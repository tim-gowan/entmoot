using System.Text.Json.Serialization;

namespace Entmoot.RoslynSolver;

internal sealed class Request
{
    [JsonPropertyName("canon")]
    public bool Canon { get; set; }
    [JsonPropertyName("root")]
    public string Root { get; set; } = "";

    [JsonPropertyName("patterns")]
    public List<string> Patterns { get; set; } = [];

    [JsonPropertyName("flags")]
    public List<string> Flags { get; set; } = [];

    [JsonPropertyName("plan")]
    public bool Plan { get; set; }
}

internal sealed class Result
{
    [JsonPropertyName("build_context")]
    public Dictionary<string, object> BuildContext { get; set; } = new(StringComparer.Ordinal);

    [JsonPropertyName("compiler_version")]
    public string CompilerVersion { get; set; } = "";

    [JsonPropertyName("calls")]
    public List<CallFact> Calls { get; set; } = [];

    [JsonPropertyName("inputs")]
    public List<string> Inputs { get; set; } = [];

    [JsonPropertyName("diagnostics")]
    public List<string> Diagnostics { get; set; } = [];

    [JsonPropertyName("invalid_environment")]
    public bool InvalidEnvironment { get; set; }
}

internal sealed class CallFact
{
    [JsonPropertyName("path")]
    public string Path { get; set; } = "";

    [JsonPropertyName("start_byte")]
    public int StartByte { get; set; }

    [JsonPropertyName("end_byte")]
    public int EndByte { get; set; }

    [JsonPropertyName("targets")]
    public List<CallTarget> Targets { get; set; } = [];

    [JsonPropertyName("owner")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public SpanRef? Owner { get; set; }

    [JsonPropertyName("gap")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? Gap { get; set; }

    [JsonPropertyName("dispatch")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public CallDispatch? Dispatch { get; set; }

    [JsonPropertyName("intrinsic")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingDefault)]
    public bool Intrinsic { get; set; }
}

internal sealed class CallDispatch
{
    [JsonPropertyName("kind")]
    public string Kind { get; set; } = "closed";

    [JsonPropertyName("policy_id")]
    public string PolicyId { get; set; } = "";

    [JsonPropertyName("evidence_pointer")]
    public string EvidencePointer { get; set; } = "/calls";

    [JsonPropertyName("assumptions")]
    public List<string> Assumptions { get; set; } = [];
}

internal sealed class CallTarget
{
    [JsonPropertyName("origin_file")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? OriginFile { get; set; }
    [JsonPropertyName("package_purl")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? PackagePurl { get; set; }
    [JsonPropertyName("package_role")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? PackageRole { get; set; }
    [JsonPropertyName("path")]
    [JsonIgnore(Condition = JsonIgnoreCondition.WhenWritingNull)]
    public string? Path { get; set; }

    [JsonPropertyName("start_byte")]
    public int StartByte { get; set; }

    [JsonPropertyName("end_byte")]
    public int EndByte { get; set; }

    [JsonPropertyName("symbol")]
    public string Symbol { get; set; } = "";

    [JsonPropertyName("kind")]
    public string Kind { get; set; } = "callable";
}

internal sealed class SpanRef
{
    [JsonPropertyName("path")]
    public string Path { get; set; } = "";

    [JsonPropertyName("start_byte")]
    public int StartByte { get; set; }

    [JsonPropertyName("end_byte")]
    public int EndByte { get; set; }
}
