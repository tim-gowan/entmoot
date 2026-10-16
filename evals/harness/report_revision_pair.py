"""Write an exclusive, read-only report from a current production revision pair.

This bounded receipt report does not resume or qualify the old longitudinal
SQLite campaign. Invoke from the repository with ``python -m``.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import blake3

from evals.harness.knowledge_sql import reader_fingerprint
from evals.harness import impact as impact_checks, revision_accounting, wire_schema
from evals.harness.revision_accounting import occurrence_accounting, summarize_occurrences, summarize_relay


def _rate(numerator: int, denominator: int) -> str:
    return f'{numerator:,}/{denominator:,} ({100 * numerator / denominator:.2f}%)' if denominator else 'ungraded (zero comparable observations)'


def write_report(base_path: Path, head_path: Path, impact_path: Path, relay_paths: list[Path],
                 output: Path, *, expected: dict | None = None,
                 caller_paths: dict[str, list[Path]] | None = None) -> Path:
    """Validate every input before creating a report; never overwrite a prior run."""
    implementation = expected if expected is not None else reader_fingerprint()['implementation']
    inputs = {}
    evaluation_inputs = {}
    for source in (Path(__file__), Path(revision_accounting.__file__), Path(impact_checks.__file__),
                   Path(wire_schema.__file__), wire_schema.CATALOG, wire_schema.QUERY_CATALOG):
        path = source.resolve(strict=True)
        evaluation_inputs[path] = 'blake3:' + blake3.blake3(path.read_bytes()).hexdigest()

    def read(path: Path) -> dict:
        resolved = path.resolve(strict=True)
        raw = resolved.read_bytes()
        value = json.loads(raw)
        inputs[resolved] = 'blake3:' + blake3.blake3(raw).hexdigest()
        return value

    base, head, impact = read(base_path), read(head_path), read(impact_path)
    counts = summarize_occurrences(occurrence_accounting(base, head, impact, expected=implementation))
    relay = summarize_relay(impact, (read(path) for path in relay_paths), expected=implementation,
                            caller_pages={address: [read(path) for path in paths]
                                          for address, paths in (caller_paths or {}).items()})
    lines = [
        '# Verified revision-pair accounting', '',
        f'Repository: `{base["snapshot"]["repository_id"]}`.',
        f'Base revision: `{base["snapshot"]["revision"]}`.',
        f'Head revision: `{head["snapshot"]["revision"]}`.',
        'Comparison topology is unqualified: these receipts alone do not prove a direct-parent relationship.', '',
        f'Occurrence union: {counts["occurrences"]:,}. Correspondence uses committed occurrence identities from both inventories.',
        f'Structural changes among comparable occurrences: {_rate(counts["structural"].get("changed", 0), counts["structural_comparable"])}.',
        f'Binding changes among comparable reference-bearing occurrences: {_rate(counts["binding"].get("changed", 0), counts["reference_comparable"])}.',
        f'Complete empty reference sets: {counts["binding"].get("complete_empty", 0):,}; excluded from the reference-bearing denominator.',
        f'Structurally incomparable occurrences: {counts["structural"].get("not_comparable", 0):,}.',
        f'Binding-incomparable occurrences: {counts["binding"].get("not_comparable", 0):,}.', '',
        'Occurrence-level identity planes:', '',
    ]
    lines.extend(f'- {key}: {counts["planes"].get(key, 0):,}' for key in
                 ('both', 'structural_only', 'resolved_only', 'neither', 'complete_empty', 'not_comparable'))
    lines.extend(['', 'Caller delivery:', '',
                  f'- Returned changed subjects: {relay["returned_subjects"]:,}/{relay["eligible_subjects"]:,}',
                  f'- Unconsumed changed subjects: {relay["unconsumed_subjects"]:,}',
                  f'- Query not run: {str(relay["query_not_run"]).lower()}'])
    for key in ('returned_callers', 'returned_witness_paths', 'subjects_with_returned_callers',
                'removed_subjects', 'missing_results', 'search_truncated_subjects',
                'subjects_with_unconsumed_callers', 'unconsumed_caller_items'):
        lines.append(f'- {key}: {relay["counts"].get(key, 0):,}')
    for status in ('known', 'unknown', 'truncated', 'conflict'):
        lines.append(f'- Subjects with caller status {status}: {relay["statuses"].get(status, 0):,}')
    lines.extend(['', 'Empty caller results do not prove absence. Search status, search limits and delivery pagination are independent.',
                  'Process success and native language-mode qualification are ungraded by this report.', '',
                  'Detailed incompleteness reasons (counts are nonexclusive):', ''])
    lines.extend(f'- `{reason}`: {count:,}' for reason, count in sorted(counts['reasons'].items()))
    lines.extend(['', 'Caller receipt gaps (per delivered page; counts are nonexclusive):', ''])
    lines.extend(f'- `{reason}`: {count:,}' for reason, count in sorted(relay['gaps'].items()))
    lines.extend(['', 'Implementation:', '', *(f'- {key}: `{value}`' for key, value in implementation.items()),
                  '', 'Evaluation implementation and schema inputs:', '',
                  *(f'- `{path}`: `{digest}`' for path, digest in evaluation_inputs.items()),
                  '', 'Input digests identify files read for this report; this report does not retain those bytes:', ''])
    for path, digest in (inputs | evaluation_inputs).items():
        # A receipt changed during evaluation cannot become a current summary.
        if 'blake3:' + blake3.blake3(path.read_bytes()).hexdigest() != digest:
            raise ValueError(f'input changed while reporting: {path}')
        if path in inputs:
            lines.append(f'- `{path}`: `{digest}`')
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open('x', encoding='utf-8', newline='\n') as stream:
        stream.write('\n'.join(lines) + '\n')
    return output


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base-graph', type=Path, required=True)
    parser.add_argument('--head-graph', type=Path, required=True)
    parser.add_argument('--impact', type=Path, required=True)
    parser.add_argument('--relay', type=Path, action='append', default=[])
    parser.add_argument('--caller-page', nargs=2, action='append', metavar=('ADDRESS_ID', 'PATH'), default=[])
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    callers: dict[str, list[Path]] = {}
    for address, path in args.caller_page:
        callers.setdefault(address, []).append(Path(path))
    print(write_report(args.base_graph, args.head_graph, args.impact, args.relay, args.out, caller_paths=callers))


if __name__ == '__main__':
    main()
