//! Translate optional scalar provider facts after snapshot fencing.
use entmoot_contracts::*;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScalarSite {
    path: String,
    start_byte: usize,
    end_byte: usize,
    sort: CanonSort,
    overflow: Option<CanonOverflow>,
}

pub(super) fn seal(raw: &mut NativeRawObservation, sources: &BTreeMap<String, String>) {
    if raw.invalid_environment || raw.budget_exceeded {
        raw.expressions.clear();
        return;
    }
    fn collect(
        context: &serde_json::Map<String, serde_json::Value>,
        prefix: &str,
        sources: &BTreeMap<String, String>,
        out: &mut Vec<CanonExpressionFact>,
    ) {
        if let Some(rows) = context
            .get("canon_expressions")
            .and_then(serde_json::Value::as_array)
        {
            for row in rows.iter().take(100_000usize.saturating_sub(out.len())) {
                let Ok(site) = serde_json::from_value::<ScalarSite>(row.clone()) else {
                    continue;
                };
                let path = if prefix.is_empty() {
                    site.path
                } else {
                    format!("{prefix}/{}", site.path)
                };
                let Some(source_digest) = sources.get(&path) else {
                    continue;
                };
                if site.start_byte >= site.end_byte
                    || site.sort == CanonSort::Unknown
                    || site.sort.validate().is_err()
                {
                    continue;
                }
                out.push(CanonExpressionFact {
                    site: NativeSpanRef {
                        path,
                        span: SiteSpan {
                            start_byte: site.start_byte,
                            end_byte: site.end_byte,
                        },
                    },
                    source_digest: ContentDigest::from_trusted(source_digest),
                    sort: site.sort,
                    overflow: site.overflow,
                    limitations: vec![],
                });
            }
        }
        if let Some(modules) = context
            .get("isolated_modules")
            .and_then(serde_json::Value::as_object)
        {
            for (prefix, module) in modules {
                if let Some(context) = module.as_object() {
                    collect(context, prefix, sources, out);
                }
            }
        }
    }
    collect(&raw.build_context, "", sources, &mut raw.expressions);
    raw.expressions.sort_by(|a, b| {
        (&a.site.path, a.site.span.start_byte, a.site.span.end_byte).cmp(&(
            &b.site.path,
            b.site.span.start_byte,
            b.site.span.end_byte,
        ))
    });
    raw.expressions.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scalar_facts_require_frozen_sources_and_a_valid_environment() {
        let mut raw = NativeRawObservation::default();
        raw.build_context.insert("canon_expressions".into(), serde_json::json!([
            {"path":"a.go","start_byte":10,"end_byte":11,"sort":{"kind":"bool"}},
            {"path":"outside.go","start_byte":10,"end_byte":11,"sort":{"kind":"bool"}},
            {"path":"a.go","start_byte":20,"end_byte":21,"sort":{"kind":"integer","bits":0,"signed":true}}
        ]));
        let digest = blake3_hex(b"fixture");
        let sources = BTreeMap::from([("a.go".into(), digest.clone())]);
        seal(&mut raw, &sources);
        assert_eq!(raw.expressions.len(), 1);
        assert_eq!(raw.expressions[0].source_digest.as_str(), digest);
        raw.invalid_environment = true;
        seal(&mut raw, &sources);
        assert!(raw.expressions.is_empty());
    }
}
