using Microsoft.Build.Locator;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.MSBuild;

namespace Entmoot.RoslynSolver;

internal sealed class AnalysisUniverse : IDisposable
{
    private readonly Workspace? _workspace;

    public AnalysisUniverse(
        Workspace? workspace,
        Solution solution,
        IReadOnlyList<string> inputs,
        IReadOnlyList<string> diagnostics,
        bool invalidEnvironment,
        Dictionary<string, object> buildContext)
    {
        _workspace = workspace;
        Solution = solution;
        Inputs = inputs;
        Diagnostics = diagnostics;
        InvalidEnvironment = invalidEnvironment;
        BuildContext = buildContext;
    }

    public Solution Solution { get; }
    public IReadOnlyList<string> Inputs { get; }
    public IReadOnlyList<string> Diagnostics { get; }
    public bool InvalidEnvironment { get; }
    public Dictionary<string, object> BuildContext { get; }

    public void Dispose() => _workspace?.Dispose();
}

internal static class WorkspaceHost
{
    public static AnalysisUniverse Load(string root)
    {
        var diagnostics = new List<string>();
        var buildContext = new Dictionary<string, object>(StringComparer.Ordinal)
        {
            ["sdk"] = Environment.Version.ToString(),
            ["target_framework"] = "net9.0",
        };

        var projects = DiscoverProjects(root);
        if (projects.Solution is not null || projects.Project is not null)
        {
            return LoadMsBuild(root, projects, buildContext, diagnostics);
        }

        return LoadAdhoc(root, buildContext, diagnostics);
    }

    private static (string? Solution, string? Project) DiscoverProjects(string root)
    {
        var solution = SourceFiles(root, "*.sln").FirstOrDefault();
        if (solution is not null)
        {
            return (solution, null);
        }

        var project = SourceFiles(root, "*.csproj").FirstOrDefault();
        return (null, project);
    }

    private static AnalysisUniverse LoadMsBuild(
        string root,
        (string? Solution, string? Project) projects,
        Dictionary<string, object> buildContext,
        List<string> diagnostics)
    {
        try
        {
            if (!MSBuildLocator.IsRegistered)
            {
                // Prefer the .NET SDK MSBuild. RegisterDefaults can bind a VS
                // instance whose hostfxr cannot resolve the installed SDK alone.
                var instance = MSBuildLocator.QueryVisualStudioInstances()
                    .Where(candidate => candidate.DiscoveryType == DiscoveryType.DotNetSdk)
                    .OrderByDescending(candidate => candidate.Version)
                    .FirstOrDefault()
                    ?? MSBuildLocator.QueryVisualStudioInstances()
                        .OrderByDescending(candidate => candidate.Version)
                        .FirstOrDefault();
                if (instance is null)
                {
                    instance = MSBuildLocator.RegisterDefaults();
                }
                else
                {
                    MSBuildLocator.RegisterInstance(instance);
                }
                buildContext["msbuild"] = instance.MSBuildPath;
                buildContext["msbuild_version"] = instance.Version.ToString();
                buildContext["msbuild_discovery"] = instance.DiscoveryType.ToString();
            }
        }
        catch (Exception ex)
        {
            diagnostics.Add($"MSBuildLocator failed: {ex.Message}");
            return Failed(diagnostics, buildContext);
        }

        // Help the out-of-proc BuildHost resolve the same SDK MSBuildLocator picked.
        var dotnetRoot = Environment.GetEnvironmentVariable("DOTNET_ROOT");
        if (string.IsNullOrWhiteSpace(dotnetRoot))
        {
            var programFiles = Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles);
            var candidate = Path.Combine(programFiles, "dotnet");
            if (Directory.Exists(candidate))
            {
                Environment.SetEnvironmentVariable("DOTNET_ROOT", candidate);
                dotnetRoot = candidate;
            }
        }
        if (!string.IsNullOrWhiteSpace(dotnetRoot))
        {
            var host = Path.Combine(dotnetRoot, OperatingSystem.IsWindows() ? "dotnet.exe" : "dotnet");
            if (File.Exists(host))
            {
                Environment.SetEnvironmentVariable("DOTNET_HOST_PATH", host);
            }
        }

        MSBuildWorkspace? workspace = null;
        var workspaceFailed = false;
        try
        {
            workspace = MSBuildWorkspace.Create();
            workspace.WorkspaceFailed += (_, args) =>
            {
                diagnostics.Add($"{args.Diagnostic.Kind}: {args.Diagnostic.Message}");
                if (args.Diagnostic.Kind == WorkspaceDiagnosticKind.Failure) workspaceFailed = true;
            };

            Solution solution;
            if (projects.Solution is not null)
            {
                buildContext["project_system"] = "msbuild_solution";
                buildContext["entry"] = PathRel(root, projects.Solution);
                solution = workspace.OpenSolutionAsync(projects.Solution).GetAwaiter().GetResult();
            }
            else
            {
                buildContext["project_system"] = "msbuild_project";
                buildContext["entry"] = PathRel(root, projects.Project!);
                var project = workspace.OpenProjectAsync(projects.Project!).GetAwaiter().GetResult();
                // A directory can contain several independent projects. Loading
                // only the first silently omitted entire compilation universes.
                foreach (var other in SourceFiles(root, "*.csproj"))
                    if (!workspace.CurrentSolution.Projects.Any(loaded =>
                        string.Equals(loaded.FilePath, other, StringComparison.OrdinalIgnoreCase)))
                        workspace.OpenProjectAsync(other).GetAwaiter().GetResult();
                solution = workspace.CurrentSolution;
                if (!string.IsNullOrWhiteSpace(project.Name))
                {
                    buildContext["project"] = project.Name;
                }

                if (project.ParseOptions is CSharpParseOptions parse)
                {
                    buildContext["language_version"] = parse.LanguageVersion.ToString();
                }
            }

            foreach (var project in solution.Projects.Where(p => p.Language == LanguageNames.CSharp))
            {
                if (project.CompilationOptions is not null)
                {
                    buildContext["output_kind"] = project.CompilationOptions.OutputKind.ToString();
                }

                break;
            }

            var inputs = CollectDocumentPaths(solution);
            var evaluated = new List<object>();
            foreach (var project in solution.Projects)
            {
                if (project.FilePath is not null) {
                    inputs.Add(project.FilePath);
                    inputs.AddRange(EvaluatedImports(project.FilePath, evaluated));
                }
                inputs.AddRange(project.MetadataReferences.OfType<PortableExecutableReference>()
                    .Select(reference => reference.FilePath).OfType<string>());
                inputs.AddRange(project.AdditionalDocuments.Concat(project.AnalyzerConfigDocuments)
                    .Select(document => document.FilePath).OfType<string>());
                inputs.AddRange(project.AnalyzerReferences.Select(reference => reference.FullPath).OfType<string>());
            }
            buildContext["evaluated_projects"] = evaluated;
            // Evaluated imports, generators and negative lookups still require
            // a build-system closure witness; listing documents is insufficient.
            buildContext["input_closure"] = "evaluated_imports_metadata_analyzers_and_additional_inputs;target_execution_and_negative_lookups_open";
            if (inputs.Count == 0)
            {
                diagnostics.Add("MSBuild workspace opened but produced no C# documents");
                workspace.Dispose();
                return Failed(diagnostics, buildContext, inputs);
            }

            return new AnalysisUniverse(
                workspace,
                solution,
                inputs,
                diagnostics,
                invalidEnvironment: workspaceFailed,
                buildContext);
        }
        catch (Exception ex)
        {
            workspace?.Dispose();
            diagnostics.Add($"MSBuild workspace load failed ({ex.GetType().Name}): {ex.Message}");
            // A discovered project owns compilation scope and references. An
            // adhoc parse cannot stand in for failed project evaluation.
            return Failed(diagnostics, buildContext,
                new[] { projects.Solution, projects.Project }.OfType<string>().ToArray());
        }
    }

    // Keep MSBuild types behind a non-inlined boundary so Locator registers the
    // installed SDK before the runtime loads its evaluation assemblies.
    [System.Runtime.CompilerServices.MethodImpl(System.Runtime.CompilerServices.MethodImplOptions.NoInlining)]
    private static IEnumerable<string> EvaluatedImports(string file, List<object> evidence)
    {
        using var collection = new Microsoft.Build.Evaluation.ProjectCollection();
        var project = collection.LoadProject(file);
        var imports = project.Imports.Select(import => import.ImportedProject.FullPath)
            .Append(project.FullPath).Distinct(StringComparer.OrdinalIgnoreCase).OrderBy(path => path, StringComparer.Ordinal).ToArray();
        evidence.Add(new {
            project = file,
            imports,
            properties = new[] { "TargetFramework", "TargetFrameworks", "LangVersion", "DefineConstants",
                "Nullable", "TargetPath", "MSBuildProjectExtensionsPath" }
                .ToDictionary(name => name, name => project.GetPropertyValue(name)),
            items = project.Items.Where(item => new[] { "Compile", "ProjectReference", "Reference",
                "Analyzer", "AdditionalFiles", "EditorConfigFiles" }.Contains(item.ItemType))
                .Select(item => new { kind = item.ItemType, include = item.EvaluatedInclude }).ToArray(),
            authority = "native_project_evaluation;not_a_target_execution_trace"
        });
        return imports;
    }

    private static AnalysisUniverse LoadAdhoc(
        string root,
        Dictionary<string, object> buildContext,
        List<string> diagnostics)
    {
        buildContext["project_system"] = "adhoc";
        buildContext["input_closure"] = "partial:adhoc_reference_profile";
        buildContext["target_framework"] = "net9.0";

        var sources = SourceFiles(root, "*.cs").ToList();

        if (sources.Count == 0)
        {
            diagnostics.Add("no .cs files found under root and no .sln/.csproj present");
            return Failed(diagnostics, buildContext);
        }

        AdhocWorkspace? workspace = null;
        try
        {
            workspace = new AdhocWorkspace();
            var projectInfo = ProjectInfo.Create(
                ProjectId.CreateNewId(),
                VersionStamp.Create(),
                name: "EntmootAdhoc",
                assemblyName: "EntmootAdhoc",
                language: LanguageNames.CSharp,
                compilationOptions: new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary),
                parseOptions: new CSharpParseOptions(LanguageVersion.Latest),
                metadataReferences: ReferenceAssemblies().ToList());

            var solution = workspace.CurrentSolution.AddProject(projectInfo);
            var projectId = projectInfo.Id;

            foreach (var path in sources)
            {
                var docId = DocumentId.CreateNewId(projectId);
                var text = File.ReadAllText(path);
                solution = solution.AddDocument(docId, Path.GetFileName(path), text, filePath: path);
            }

            if (!workspace.TryApplyChanges(solution))
            {
                diagnostics.Add("AdhocWorkspace failed to apply document set");
                workspace.Dispose();
                return Failed(diagnostics, buildContext, sources);
            }

            return new AnalysisUniverse(
                workspace,
                workspace.CurrentSolution,
                sources,
                diagnostics,
                invalidEnvironment: false,
                buildContext);
        }
        catch (Exception ex)
        {
            workspace?.Dispose();
            diagnostics.Add($"Adhoc compilation setup failed: {ex.Message}");
            return Failed(diagnostics, buildContext, sources);
        }
    }

    private static AnalysisUniverse Failed(
        List<string> diagnostics,
        Dictionary<string, object> buildContext,
        IReadOnlyList<string>? inputs = null)
    {
        var empty = new AdhocWorkspace();
        return new AnalysisUniverse(
            empty,
            empty.CurrentSolution,
            inputs ?? [],
            diagnostics,
            invalidEnvironment: true,
            buildContext);
    }

    private static IEnumerable<MetadataReference> ReferenceAssemblies() =>
        Basic.Reference.Assemblies.Net90.References.All;

    private static List<string> CollectDocumentPaths(Solution solution) =>
        solution.Projects
            .Where(project => project.Language == LanguageNames.CSharp)
            .SelectMany(project => project.Documents)
            .Select(document => document.FilePath)
            .Where(path => !string.IsNullOrWhiteSpace(path))
            .Cast<string>()
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .OrderBy(path => path, StringComparer.OrdinalIgnoreCase)
            .ToList();

    private static string PathRel(string root, string path) =>
        Path.GetRelativePath(root, path).Replace('\\', '/');

    private static IEnumerable<string> SourceFiles(string root, string pattern)
    {
        var pending = new Stack<string>();
        var files = new List<string>();
        pending.Push(root);
        while (pending.TryPop(out var directory))
        {
            foreach (var child in Directory.EnumerateDirectories(directory))
            {
                var name = Path.GetFileName(child);
                if ((name.StartsWith('.') && name != ".github")
                    || new[] { "bin", "obj", "target", "build", "dist", "node_modules", "venv", "__pycache__", "coverage", "htmlcov" }.Contains(name, StringComparer.OrdinalIgnoreCase)) continue;
                if ((File.GetAttributes(child) & FileAttributes.ReparsePoint) != 0)
                    throw new IOException($"linked C# source directory: {child}");
                pending.Push(child);
            }
            foreach (var file in Directory.EnumerateFiles(directory, pattern))
            {
                if ((File.GetAttributes(file) & FileAttributes.ReparsePoint) != 0)
                    throw new IOException($"linked C# source file: {file}");
                files.Add(file);
            }
        }
        return files.OrderBy(path => path, StringComparer.Ordinal);
    }
}
