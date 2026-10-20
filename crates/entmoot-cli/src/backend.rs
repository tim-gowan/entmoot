//! Explicit MCP client for the shared read service. Never starts a backend.
use super::{AssessCommand, Commands};
use anyhow::{bail, Context, Result};
use rmcp::{
    model::CallToolRequestParams,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    ServiceExt,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn request<T: DeserializeOwned + Default>(path: &Option<PathBuf>) -> Result<T> {
    path.as_ref().map_or_else(
        || Ok(T::default()),
        |path| Ok(serde_json::from_slice(&std::fs::read(path)?)?),
    )
}
fn value(value: impl Serialize) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}
fn configured(paths: &[&Option<PathBuf>]) -> Result<()> {
    if paths.iter().any(|p| p.is_some()) {
        bail!("backend reads use the server's configured publication; remove local graph/store/source options, or use --standalone");
    }
    Ok(())
}

pub(super) fn run(command: &Commands, url: &str, token_file: Option<&Path>) -> Result<bool> {
    let mut complete = false;
    let mut out = None;
    let calls: Vec<(&str,Value)> = match command {
        Commands::Request { .. }
        | Commands::Status
        | Commands::Inventory { .. }
        | Commands::CallComponents { .. }
        | Commands::Find(_)
        | Commands::Inspect(_)
        | Commands::ReadSource(_)
        | Commands::Callers(_)
        | Commands::Callees(_)
        | Commands::Boundaries(_)
        | Commands::Neighborhood(_)
        | Commands::Path(_)
        | Commands::RelayImpact { .. }
        | Commands::TraceSeededPaths { .. } => return Ok(false),
        Commands::Assess { command: AssessCommand::Clones { graph,publication,store,source_root,request:path } } => {
            configured(&[graph,publication,store,source_root])?;
            vec![("assess_clones",value(request::<entmoot_contracts::CloneRequest>(path)?)?)]
        }
        Commands::Assess { command: AssessCommand::Unused { graph,publication,store,source_root,request:path,require_complete } } => {
            configured(&[graph,publication,store,source_root])?;
            complete= *require_complete;
            vec![("assess_unused",value(request::<entmoot_contracts::UnusedRequest>(path)?)?)]
        }
        Commands::Assess { command: AssessCommand::Compatibility { consumer,provider,out:destination } } => {
            configured(&[consumer])?;
            if !provider.is_empty() { bail!("configure provider publications when starting the backend"); }
            out=destination.as_deref();vec![("compatibility",json!({}))]
        }
        Commands::Assess { command: AssessCommand::Impact(args) } => {
            configured(&[&args.base_graph,&args.head_graph])?;
            if args.revalidate || args.config!="v1" { bail!("backend impact uses server configuration; use --standalone for a local revalidation package or policy"); }
            out=args.out.as_deref();
            if args.tests { vec![("assess_test_impact",value(request::<entmoot_contracts::TestImpactRequest>(&args.test_targets)?)?)] } else { vec![("impact",json!({}))] }
        }
        Commands::Query { .. } => bail!("expert store reads remain standalone; use --standalone or the portable request operation with the backend"),
        _ => return Ok(false),
    };
    let url = reqwest::Url::parse(url).context("invalid backend URL")?;
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "http" || !loopback || !url.username().is_empty() || url.password().is_some()
    {
        bail!("backend URL must be an http loopback endpoint without embedded credentials");
    }
    let token = match token_file {
        Some(path) => std::fs::read_to_string(path)?,
        None => std::env::var("ENTMOOT_QUERY_TOKEN")
            .context("set ENTMOOT_QUERY_TOKEN or --backend-token-file")?,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let results = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(300))
            .build()?;
        let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string())
            .auth_header(token.trim());
        config.allow_stateless = true;
        config.max_sse_event_size = 64 * 1024 * 1024;
        let transport = StreamableHttpClientTransport::with_client(client, config);
        let service = ()
            .serve(transport)
            .await
            .context("connect to query backend; start entmoot-mcp --listen explicitly")?;
        let mut results = Vec::new();
        for (name, args) in calls {
            let response = service
                .call_tool(
                    CallToolRequestParams::new(name.to_owned()).with_arguments(
                        args.as_object()
                            .context("tool arguments must be an object")?
                            .clone(),
                    ),
                )
                .await?;
            let value = response
                .structured_content
                .context("backend omitted its structured result")?;
            if response.is_error == Some(true) {
                bail!("backend {}: {}", value["kind"], value["message"]);
            }
            results.push(value);
        }
        service.cancel().await?;
        Ok::<_, anyhow::Error>(results)
    })?;
    let result = results
        .into_iter()
        .next()
        .context("backend returned no result")?;
    if let Some(nudge) = result["nudge"].as_str() {
        eprintln!("{nudge}");
    }
    if let Some(path) = out {
        entmoot_workspace::workspace::write_json(path, &result)?;
    }
    crate::output::emit(&result)?;
    if complete
        && !serde_json::from_value::<entmoot_contracts::UnusedReport>(result)?
            .completeness
            .established()
    {
        bail!("completeness not established");
    }
    Ok(true)
}
