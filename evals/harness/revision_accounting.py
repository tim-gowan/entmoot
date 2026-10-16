"""Independent accounting over current production revision and caller receipts.

These are evaluation summaries, never graph facts. Input wire shapes belong to
Rust. No inferred correspondence, historical-wire aliases or first-producer
selection is permitted here. Each pair is checked before campaign aggregation.
"""
from __future__ import annotations

from collections import Counter
from dataclasses import dataclass
import json
from typing import Any, Iterable

import blake3

from evals.harness.contracts import ContractError
from evals.harness.impact import verify_inventory_delta, verify_breaking_change_signals
from evals.harness.wire_schema import validate_wire


@dataclass(frozen=True)
class OccurrenceAccounting:
    address_id: str
    logic_address: str
    structural: str
    structural_reasons: tuple[str, ...]
    binding: str
    binding_reasons: tuple[str, ...]
    # Lossless attributable records, not a selected producer's digest.
    base_records: tuple[dict, ...]
    head_records: tuple[dict, ...]


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def _occurrences(graph: dict) -> dict[str, dict]:
    result: dict[str, dict] = {}
    for node in graph['nodes']:
        if node['kind'] != 'callable':
            continue
        address, envelope = node.get('address_id'), node.get('logic_address')
        _require(isinstance(envelope, str) and envelope.startswith('logic|'),
                 'callable inventory lacks a structural occurrence envelope')
        _require(address == blake3.blake3(envelope.encode()).hexdigest(),
                 'callable inventory occurrence commitment mismatch')
        _require(address not in result, 'duplicate callable occurrence')
        result[address] = node
    return result


def verify_context(base: dict, head: dict, impact: dict, expected: dict) -> None:
    """Refuse old/mixed receipts before interpreting any comparison outcome."""
    validate_wire(expected, 'ImplementationFingerprint')
    validate_wire(impact, 'RevisionImpactManifest')
    _require(impact['schema_version'] == 'entmoot.revision_impact.v1', 'unsupported impact series')
    for name, graph in (('base', base), ('head', head)):
        _require(graph.get('schema_version') == 'logic.graph.v1', 'unsupported graph series')
        _require(graph.get('implementation') == expected, f'{name} implementation mismatch')
        _require(graph['snapshot'] == impact[f'{name}_snapshot'], f'{name} snapshot mismatch')
        _require(graph['policy_ids'] == impact[f'{name}_policy_ids'], f'{name} policy attribution mismatch')
    _require(impact['implementation'] == expected, 'impact implementation mismatch')
    _require(base['snapshot']['repository_id'] == head['snapshot']['repository_id'],
             'repository namespace mismatch')
    inventories = [_occurrences(graph) for graph in (base, head)]
    for graph, inventory in zip((base, head), inventories):
        for record in graph.get('resolved_identities', []):
            _require(record['address_id'] in inventory, 'binding lacks an inventoried occurrence')
    verify_inventory_delta(base, head, impact)
    verify_breaking_change_signals(base, head, impact)


def occurrence_accounting(base: dict, head: dict, impact: dict, *, expected: dict
                          ) -> Iterable[OccurrenceAccounting]:
    """Yield the exact occurrence union; missing/empty bindings stay separate.

    Scope gaps suppress structural equality as well as structural change claims.
    Binding comparison follows the independently checked producer-series result;
    complete zero-reference observations are separated from reference coverage.
    """
    verify_context(base, head, impact, expected)
    left, right = _occurrences(base), _occurrences(head)
    bindings = {row['address_id']: row for row in impact['bindings']}
    _require(set(bindings) <= left.keys() | right.keys(), 'binding outside occurrence union')
    failures = [{row.get('provider_pointer') for row in graph.get('diagnostics', [])
                 if row.get('code', '').startswith('source.')} for graph in (base, head)]
    policy_equal = bool(base['policy_ids']) and base['policy_ids'] == head['policy_ids']
    for address in sorted(left.keys() | right.keys()):
        a, b = left.get(address), right.get(address)
        reasons = []
        if a is None:
            reasons.append('missing_base_occurrence')
        if b is None:
            reasons.append('missing_head_occurrence')
        if not policy_equal:
            reasons.append('structural_policy_set_not_comparable')
        for name, node, gaps in (('base', a, failures[0]), ('head', b, failures[1])):
            if node and not node.get('structural_hash'):
                reasons.append(f'{name}_structural_hash_missing')
            # A missing side may be caused by a failed parse of the shared path.
            if (node or a or b)['path'] in gaps:
                reasons.append(f'{name}_source_inventory_incomplete')
        structural = 'not_comparable' if reasons else (
            'unchanged' if a['structural_hash'] == b['structural_hash'] else 'changed')
        row = bindings.get(address)
        if row is None:
            binding, binding_reasons, before, after = 'not_comparable', ['missing_binding_evidence'], [], []
        else:
            before, after = row['base'], row['head']
            binding, binding_reasons = row['kind'], list(row['reasons'])
            if binding != 'not_comparable' and all(
                record['binding']['reference_count'] == 0 for record in before + after
            ):
                binding = 'complete_empty'
            # A producer's reported reason is not replaced by the coarse impact reason.
            for side, records in (('base', before), ('head', after)):
                for record in records:
                    binding_reasons.extend(f'{side}:{reason}' for reason in record['binding'].get('reason_codes', []))
        yield OccurrenceAccounting(address, (a or b)['logic_address'], structural,
                                   tuple(reasons), binding, tuple(binding_reasons),
                                   tuple(before), tuple(after))


def summarize_occurrences(rows: Iterable[OccurrenceAccounting]) -> dict[str, Any]:
    """Stream one checked pair's rows. Never join different occurrences by commit."""
    structural, binding, planes, reasons = Counter(), Counter(), Counter(), Counter()
    count = 0
    for row in rows:
        count += 1
        structural[row.structural] += 1
        binding[row.binding] += 1
        reasons.update(row.structural_reasons)
        reasons.update(row.binding_reasons)
        if row.structural == 'not_comparable' or row.binding == 'not_comparable':
            planes['not_comparable'] += 1
        elif row.binding == 'complete_empty':
            planes['complete_empty'] += 1
        else:
            planes[('both' if row.binding == 'changed' else 'structural_only')
                   if row.structural == 'changed'
                   else ('resolved_only' if row.binding == 'changed' else 'neither')] += 1
    return {'occurrences': count, 'structural': dict(structural), 'binding': dict(binding),
            'planes': dict(planes), 'reasons': dict(reasons),
            'reference_comparable': binding['changed'] + binding['unchanged'],
            'structural_comparable': structural['changed'] + structural['unchanged']}


def _page(page: dict, items: list, *, offset: int, total: int) -> int:
    _require(page['offset'] == offset and page['total_items'] == total,
             'noncontiguous or inconsistent output page')
    _require(page['returned'] == len(items), 'output returned count mismatch')
    end = offset + len(items)
    _require(end <= total, 'output page exceeds total')
    cursor = page.get('next_cursor')
    _require((cursor is not None) == (end < total), 'output cursor does not account for remaining items')
    if cursor is not None:
        _require(cursor['offset'] == end and end > offset, 'nonprogressing output cursor')
    return end


def summarize_relay(impact: dict, pages: Iterable[dict], *, expected: dict,
                    caller_pages: dict[str, list[dict]] | None = None) -> dict[str, Any]:
    """Account for delivered pages; never call missing/truncated work a caller.

    Additional caller pages are keyed by the actual changed occurrence address.
    An unfinished delivery stays explicit even when the search itself completed.
    The returned endpoint count is distinct from represented witness-path count.
    """
    validate_wire(impact, 'RevisionImpactManifest')
    _require(impact['implementation'] == expected, 'impact implementation mismatch')
    changed = {row['occurrence']['address_id']: row for row in impact['callables']}
    _require(len(changed) == len(impact['callables']), 'duplicate changed occurrence')
    extra = caller_pages or {}
    used_extra: set[str] = set()
    seen: set[str] = set()
    offset, page_count, request = 0, 0, None
    counts, statuses, gaps = Counter(), Counter(), Counter()
    for page in pages:
        validate_wire(page, 'ImpactRelayResult')
        _require(page['schema_version'] == 'entmoot.impact_relay.v1', 'unsupported relay series')
        _require(page['implementation'] == expected, 'relay implementation mismatch')
        for side in ('base', 'head'):
            _require(page[f'{side}_snapshot'] == impact[f'{side}_snapshot'], 'relay snapshot mismatch')
        if request is None:
            request = page['request']
        _require(page['request'] == request, 'relay request changed across pages')
        offset = _page(page['page'], page['items'], offset=offset, total=len(changed))
        page_count += 1
        gaps.update(page['gaps'])
        for item in page['items']:
            address = item['callable']['occurrence']['address_id']
            _require(address not in seen and changed.get(address) == item['callable'],
                     'relay changed occurrence missing, duplicated or misattributed')
            seen.add(address)
            gaps.update(item['gaps'])
            first = item.get('callers')
            if first is None:
                _require(bool(item['gaps']), 'missing caller result has no limitation')
                counts['removed_subjects' if item['callable']['kind'] == 'removed' else 'missing_results'] += 1
                continue
            more = []
            for envelope in extra.get(address, []):
                # Continuations are ordinary CLI QueryResponse envelopes; the
                # first page is the compact payload embedded by the relay.
                validate_wire(envelope, 'QueryResponse')
                _require(envelope.get('tier') == 'compact', 'caller continuation is not compact')
                more.append({key: value for key, value in envelope.items() if key != 'tier'})
            if address in extra:
                used_extra.add(address)
            cursor_offset, total, endpoints = 0, first['page']['total_items'], set()
            witnesses: set[str] = set()
            counts['search_truncated_subjects'] += bool(first['search_truncated'])
            statuses[first['status']] += 1
            for caller in [first, *more]:
                validate_wire({'tier': 'compact', **caller}, 'QueryResponse')
                _require(caller['snapshot'] == impact['head_snapshot'], 'caller snapshot mismatch')
                query = caller['query']
                _require(query == first['query'], 'caller query changed across pages')
                _require(query.get('operation') == 'traverse'
                         and query['request'].get('kind') == 'neighborhood'
                         and query['request'].get('direction') == 'callers'
                         and query['request'].get('start') == item['callable'].get('head_node_id'),
                         'caller query does not target the changed head occurrence')
                for field in ('status', 'search_truncated', 'result_bounds', 'gaps'):
                    _require(caller[field] == first[field], 'caller search meaning changed across pages')
                cursor_offset = _page(caller['page'], caller['items'], offset=cursor_offset, total=total)
                gaps.update(caller['gaps'])
                for witness in caller['items']:
                    _require(witness['kind'] == 'path' and witness.get('endpoint') in caller['subjects'],
                             'caller item lacks an exact represented endpoint')
                    _require(all(edge in caller['edges'] for edge in witness['edges']),
                             'caller witness lacks represented edges')
                    key = json.dumps(witness, sort_keys=True)
                    _require(key not in witnesses, 'duplicate caller witness across delivery pages')
                    witnesses.add(key)
                    endpoints.add(witness['endpoint'])
                    counts['returned_witness_paths'] += 1
            counts['returned_callers'] += len(endpoints)
            counts['subjects_with_returned_callers'] += bool(endpoints)
            counts['unconsumed_caller_items'] += total - cursor_offset
            counts['subjects_with_unconsumed_callers'] += cursor_offset < total
    _require(used_extra == set(extra), 'caller continuation has no delivered subject')
    return {'pages': page_count, 'eligible_subjects': len(changed),
            'returned_subjects': len(seen), 'unconsumed_subjects': len(changed) - len(seen),
            'query_not_run': page_count == 0, 'counts': dict(counts),
            'statuses': dict(statuses), 'gaps': dict(gaps)}
