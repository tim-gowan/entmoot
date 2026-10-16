using System.Text.Json;
using System.Text.Json.Serialization;
using Microsoft.CodeAnalysis.CSharp;

namespace Entmoot.RoslynSolver;

internal static class Program
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = null,
        DefaultIgnoreCondition = JsonIgnoreCondition.Never,
        WriteIndented = false,
    };

    public static int Main()
    {
        Request request;
        try
        {
            request = JsonSerializer.Deserialize<Request>(Console.In.ReadToEnd(), JsonOptions)
                      ?? throw new InvalidOperationException("request JSON deserialized to null");
        }
        catch (Exception ex)
        {
            WriteResult(new Result
            {
                CompilerVersion = CompilerVersion(),
                InvalidEnvironment = true,
                Diagnostics = [$"invalid request JSON: {ex.Message}"],
            });
            return 0;
        }

        if (string.IsNullOrWhiteSpace(request.Root))
        {
            WriteResult(new Result
            {
                CompilerVersion = CompilerVersion(),
                InvalidEnvironment = true,
                Diagnostics = ["root is required"],
            });
            return 0;
        }

        string root;
        try
        {
            root = Path.GetFullPath(request.Root);
        }
        catch (Exception ex)
        {
            WriteResult(new Result
            {
                CompilerVersion = CompilerVersion(),
                InvalidEnvironment = true,
                Diagnostics = [$"invalid root path: {ex.Message}"],
            });
            return 0;
        }

        if (!Directory.Exists(root))
        {
            WriteResult(new Result
            {
                CompilerVersion = CompilerVersion(),
                InvalidEnvironment = true,
                Diagnostics = [$"root directory does not exist: {root}"],
            });
            return 0;
        }

        using var universe = WorkspaceHost.Load(root);
        var diagnostics = universe.Diagnostics.ToList();
        var result = new Result
        {
            BuildContext = universe.BuildContext.ToDictionary(pair => pair.Key, pair => (object)pair.Value),
            CompilerVersion = CompilerVersion(),
            Inputs = universe.Inputs.OrderBy(p => p, StringComparer.Ordinal).ToList(),
            InvalidEnvironment = universe.InvalidEnvironment,
        };

        if (!request.Plan && !universe.InvalidEnvironment)
        {
            try {
                var api = PackageApi.Load(result);
                result.Inputs.AddRange(AppDomain.CurrentDomain.GetAssemblies().Where(assembly => !assembly.IsDynamic)
                    .Select(assembly => assembly.Location).Where(File.Exists));
                var fence = new InputFence(result.Inputs);
                var canon = request.Canon ? new List<object>() : null;
                result.Calls = CallCollector.Collect(universe.Solution, root, diagnostics, api, fence, canon);
                if (canon != null) result.BuildContext["canon_expressions"] = canon;
                fence.Verify();
                result.BuildContext["input_digests"] = fence.Digests;
                result.BuildContext["input_directory_entries"] = fence.Directories;
                result.BuildContext["package_api"] = api.Result();
            } catch (Exception error) {
                result.InvalidEnvironment = true;
                diagnostics.Add("native collection failed: " + error.Message);
            }
        }

        result.Diagnostics = diagnostics
            .Distinct(StringComparer.Ordinal)
            .OrderBy(d => d, StringComparer.Ordinal)
            .ToList();

        WriteResult(result);
        return 0;
    }

    private static string CompilerVersion()
    {
        var version = typeof(CSharpCompilation).Assembly.GetName().Version?.ToString();
        return string.IsNullOrWhiteSpace(version) ? "unknown" : version;
    }

    private static void WriteResult(Result result)
    {
        Console.Out.Write(JsonSerializer.Serialize(result, JsonOptions));
        Console.Out.Write('\n');
    }
}
