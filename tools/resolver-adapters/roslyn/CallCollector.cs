using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

namespace Entmoot.RoslynSolver;

internal static class CallCollector
{
    private static readonly SymbolDisplayFormat SymbolFormat = new(
        globalNamespaceStyle: SymbolDisplayGlobalNamespaceStyle.Omitted,
        typeQualificationStyle: SymbolDisplayTypeQualificationStyle.NameAndContainingTypesAndNamespaces,
        genericsOptions: SymbolDisplayGenericsOptions.IncludeTypeParameters,
        memberOptions:
            SymbolDisplayMemberOptions.IncludeContainingType
            | SymbolDisplayMemberOptions.IncludeParameters,
        parameterOptions:
            SymbolDisplayParameterOptions.IncludeType
            | SymbolDisplayParameterOptions.IncludeParamsRefOut,
        miscellaneousOptions:
            SymbolDisplayMiscellaneousOptions.UseSpecialTypes
            | SymbolDisplayMiscellaneousOptions.EscapeKeywordIdentifiers);

    public static List<CallFact> Collect(Solution solution, string root, List<string> diagnostics, PackageApi api, InputFence fence, List<object>? canon)
    {
        var calls = new List<CallFact>();
        var offsetCache = new Dictionary<SyntaxTree, Utf8OffsetMap>();

        foreach (var project in solution.Projects.Where(p => p.Language == LanguageNames.CSharp)
                     .OrderBy(p => p.Name, StringComparer.Ordinal))
        {
            Compilation? compilation;
            try
            {
                compilation = project.GetCompilationAsync().GetAwaiter().GetResult();
            }
            catch (Exception ex)
            {
                diagnostics.Add($"compilation failed for project {project.Name}: {ex.Message}");
                continue;
            }

            if (compilation is null)
            {
                diagnostics.Add($"compilation unavailable for project {project.Name}");
                continue;
            }

            var compilationErrors = compilation.GetDiagnostics()
                         .Where(d => d.Severity == DiagnosticSeverity.Error)
                         .OrderBy(d => d.ToString(), StringComparer.Ordinal).ToArray();
            foreach (var diagnostic in compilationErrors)
            {
                diagnostics.Add(diagnostic.ToString());
            }

            foreach (var tree in compilation.SyntaxTrees.OrderBy(t => t.FilePath, StringComparer.OrdinalIgnoreCase))
            {
                fence.CheckTree(tree);
                var relativePath = RelativeToRoot(root, tree.FilePath);
                if (string.IsNullOrEmpty(relativePath))
                {
                    continue;
                }

                var model = compilation.GetSemanticModel(tree);
                var map = GetMap(tree, offsetCache);
                var rootNode = tree.GetRoot();

                foreach (var node in rootNode.DescendantNodes())
                {
                    if (compilationErrors.Length == 0 && node is BaseTypeDeclarationSyntax typeSyntax
                        && model.GetDeclaredSymbol(typeSyntax) is INamedTypeSymbol exportedType)
                    {
                        var (typeStart, typeEnd) = map.ToBytes(typeSyntax.Span);
                        api.ExportType(exportedType, new CallTarget { Path = relativePath, StartByte = typeStart,
                            EndByte = typeEnd, Symbol = exportedType.ToDisplayString(SymbolFormat), Kind = "type" });
                    }
                    if (compilationErrors.Length == 0 && node is MethodDeclarationSyntax declaration && model.GetDeclaredSymbol(declaration) is IMethodSymbol exported
                        && TryEmitTarget(exported, compilation, root, offsetCache, out var exportTarget))
                        api.Export(exported, exportTarget);
                    if (canon is { Count: < 100000 } && compilationErrors.Length == 0 && node is ExpressionSyntax or ParameterSyntax or VariableDeclaratorSyntax) {
                        ITypeSymbol? type = null;
                        if (node is ExpressionSyntax expression) {
                            var typeInfo = model.GetTypeInfo(expression);
                            if (SymbolEqualityComparer.Default.Equals(typeInfo.Type, typeInfo.ConvertedType)) type = typeInfo.Type;
                        } else if (model.GetDeclaredSymbol(node) is IParameterSymbol parameter) type = parameter.Type;
                        else if (model.GetDeclaredSymbol(node) is ILocalSymbol local) type = local.Type;
                        object? sort = type?.SpecialType switch {
                            SpecialType.System_Boolean => new { kind = "bool" },
                            SpecialType.System_SByte => new { kind = "integer", bits = 8, signed = true },
                            SpecialType.System_Byte => new { kind = "integer", bits = 8, signed = false },
                            SpecialType.System_Int16 => new { kind = "integer", bits = 16, signed = true },
                            SpecialType.System_UInt16 => new { kind = "integer", bits = 16, signed = false },
                            SpecialType.System_Int32 => new { kind = "integer", bits = 32, signed = true },
                            SpecialType.System_UInt32 => new { kind = "integer", bits = 32, signed = false },
                            SpecialType.System_Int64 => new { kind = "integer", bits = 64, signed = true },
                            SpecialType.System_UInt64 => new { kind = "integer", bits = 64, signed = false },
                            _ => null
                        };
                        if (sort != null) {
                            var (start, end) = map.ToBytes(node.Span);
                            string? overflow = model.GetOperation(node) is Microsoft.CodeAnalysis.Operations.IBinaryOperation binary
                                && binary.OperatorMethod == null ? (binary.IsChecked ? "checked" : "wrap") : null;
                            canon.Add(new { path = relativePath, start_byte = start, end_byte = end, sort, overflow });
                        }
                    }
                    if (node is not (InvocationExpressionSyntax or ObjectCreationExpressionSyntax))
                    {
                        continue;
                    }

                    var call = BuildCall(node, model, map, relativePath, root, offsetCache);
                    if (compilationErrors.Length == 0 && model.GetSymbolInfo(node).Symbol is IMethodSymbol method) api.Reference(method, call);
                    calls.Add(call);
                }
            }
        }

        calls.Sort(static (a, b) =>
        {
            var path = string.CompareOrdinal(a.Path, b.Path);
            return path != 0 ? path : a.StartByte.CompareTo(b.StartByte);
        });

        return calls;
    }

    private static CallFact BuildCall(
        SyntaxNode node,
        SemanticModel model,
        Utf8OffsetMap map,
        string relativePath,
        string root,
        Dictionary<SyntaxTree, Utf8OffsetMap> offsetCache)
    {
        var (start, end) = map.ToBytes(node.Span);
        var fact = new CallFact
        {
            Path = relativePath,
            StartByte = start,
            EndByte = end,
            Owner = FindOwner(node, model, root, offsetCache),
        };

        var info = model.GetSymbolInfo(node);
        // CandidateSymbols are binding ambiguity, not proven runtime dispatch.
        if (info.Symbol is null && info.CandidateSymbols.Length > 0)
        {
            foreach (var candidate in info.CandidateSymbols)
            {
                if (TryEmitTarget(candidate, model.Compilation, root, offsetCache, out var target))
                {
                    fact.Targets.Add(target);
                }
            }

            if (fact.Targets.Count == 0)
            {
                fact.Gap = "unresolved";
            }

            return fact;
        }

        if (info.Symbol is null)
        {
            fact.Gap = "unresolved";
            return fact;
        }

        if (info.Symbol is IMethodSymbol { MethodKind: MethodKind.DelegateInvoke })
        {
            fact.Gap = "indirect_call";
            return fact;
        }

        if (info.Symbol is IMethodSymbol method)
        {
            if (TryEmitTarget(method, model.Compilation, root, offsetCache, out var target))
            {
                fact.Targets.Add(target);
                // A unique compiler declaration does not close overriding at runtime.
                // Explicit base calls use non-virtual dispatch.
                var baseCall = node is InvocationExpressionSyntax {
                    Expression: MemberAccessExpressionSyntax { Expression: BaseExpressionSyntax } };
                if (!baseCall && !method.IsStatic && !method.IsSealed
                    && !method.ContainingType.IsSealed
                    && (method.IsVirtual || method.IsAbstract || method.IsOverride
                        || method.ContainingType.TypeKind == TypeKind.Interface))
                    fact.Gap = "dynamic_dispatch";
                else if (fact.Gap is null && fact.Owner is not null && fact.Targets.Count == 1
                    && target.Kind == "callable")
                {
                    var (policy, fence) = ClosedPolicy(method, baseCall);
                    fact.Dispatch = new CallDispatch
                    {
                        Kind = "closed",
                        PolicyId = policy,
                        EvidencePointer = "/calls",
                        Assumptions =
                        [
                            "selected MSBuild project language version and references",
                            fence,
                        ],
                    };
                }
            }
            else
            {
                fact.Gap = "unresolved";
            }

            return fact;
        }

        // Property/event-like invoke surfaces and other non-method callables.
        fact.Gap = "indirect_call";
        return fact;
    }

    private static bool TryEmitTarget(
        ISymbol symbol,
        Compilation compilation,
        string root,
        Dictionary<SyntaxTree, Utf8OffsetMap> offsetCache,
        out CallTarget target)
    {
        target = new CallTarget();

        if (symbol is not IMethodSymbol method)
        {
            return false;
        }

        if (method.MethodKind == MethodKind.DelegateInvoke)
        {
            return false;
        }

        method = (method.ReducedFrom ?? method).OriginalDefinition;
        // A partial definition can name a separate implementation. Only that
        // compiler-selected implementation has an executable source body.
        method = method.PartialImplementationPart ?? method;
        target.Symbol = method.ToDisplayString(SymbolFormat);
        target.Kind = "callable";

        var declaration = method.DeclaringSyntaxReferences.FirstOrDefault();
        // Implicit constructors have no method syntax; anchor on the type declaration.
        if (declaration is null
            && method.MethodKind == MethodKind.Constructor
            && method.ContainingType is not null)
        {
            declaration = method.ContainingType.DeclaringSyntaxReferences.FirstOrDefault();
        }

        if (declaration is null)
        {
            var origins = compilation.References.OfType<PortableExecutableReference>()
                .Where(reference => SymbolEqualityComparer.Default.Equals(
                    compilation.GetAssemblyOrModuleSymbol(reference), method.ContainingAssembly))
                .Select(reference => reference.FilePath).Where(path => path is not null).Distinct().ToArray();
            if (origins.Length == 1) target.OriginFile = origins[0];
            // Metadata-only (BCL / external) — keep symbol, no path.
            target.StartByte = 0;
            target.EndByte = 0;
            return true;
        }

        var syntax = declaration.GetSyntax();
        // Implicit constructors are represented by a type declaration, not a
        // fabricated callable spanning the whole class.
        if (syntax is BaseTypeDeclarationSyntax) target.Kind = "type";
        else if (method.IsAbstract || method.IsExtern
                 || syntax is BaseMethodDeclarationSyntax { Body: null, ExpressionBody: null })
            target.Kind = "callable_signature";
        var tree = declaration.SyntaxTree;
        target.OriginFile = System.IO.Path.GetFullPath(tree.FilePath);
        var map = GetMap(tree, offsetCache);
        var (start, end) = map.ToBytes(syntax.Span);
        var path = RelativeToRoot(root, tree.FilePath);
        target.Path = string.IsNullOrEmpty(path) ? null : path;
        target.StartByte = start;
        target.EndByte = end;
        return true;
    }

    private static SpanRef? FindOwner(
        SyntaxNode node,
        SemanticModel model,
        string root,
        Dictionary<SyntaxTree, Utf8OffsetMap> offsetCache)
    {
        var enclosing = model.GetEnclosingSymbol(node.SpanStart);
        while (enclosing is not null)
        {
            if (enclosing is IMethodSymbol method
                && method.MethodKind is MethodKind.Ordinary
                    or MethodKind.LocalFunction
                    or MethodKind.Constructor
                    or MethodKind.StaticConstructor
                    or MethodKind.PropertyGet
                    or MethodKind.PropertySet
                    or MethodKind.EventAdd
                    or MethodKind.EventRemove)
            {
                var declaration = method.DeclaringSyntaxReferences.FirstOrDefault();
                if (declaration is null)
                {
                    break;
                }

                var syntax = declaration.GetSyntax();
                var map = GetMap(declaration.SyntaxTree, offsetCache);
                var (start, end) = map.ToBytes(syntax.Span);
                var path = RelativeToRoot(root, declaration.SyntaxTree.FilePath);
                if (string.IsNullOrEmpty(path))
                {
                    break;
                }

                return new SpanRef
                {
                    Path = path,
                    StartByte = start,
                    EndByte = end,
                };
            }

            enclosing = enclosing.ContainingSymbol;
        }

        return null;
    }

    private static (string Policy, string Fence) ClosedPolicy(IMethodSymbol method, bool baseCall)
    {
        if (baseCall)
        {
            return (
                "entmoot.csharp.direct-base.v1",
                "source-level base call with non-virtual dispatch; foreign and interface replacement are outside this qualification");
        }

        var implemented = method.PartialImplementationPart ?? method;
        if (method.IsPartialDefinition || !SymbolEqualityComparer.Default.Equals(implemented, method)
            || implemented.IsPartialDefinition)
        {
            return (
                "entmoot.csharp.direct-partial.v1",
                "compiler-selected partial method implementation; foreign and interface replacement are outside this qualification");
        }

        if (method.IsStatic)
        {
            return (
                "entmoot.csharp.direct-static.v1",
                "source-level static method invocation; foreign and interface replacement are outside this qualification");
        }

        if (method.IsSealed || method.ContainingType.IsSealed)
        {
            return (
                "entmoot.csharp.direct-sealed.v1",
                "source-level sealed method or sealed-type instance call; foreign and open virtual replacement are outside this qualification");
        }

        return (
            "entmoot.csharp.direct-instance.v1",
            "source-level non-virtual instance method; foreign, virtual, and interface replacement are outside this qualification");
    }

    private static Utf8OffsetMap GetMap(SyntaxTree tree, Dictionary<SyntaxTree, Utf8OffsetMap> cache)
    {
        if (cache.TryGetValue(tree, out var existing))
        {
            return existing;
        }

        var map = Utf8OffsetMap.FromText(tree.GetText(), tree.FilePath);
        cache[tree] = map;
        return map;
    }

    private static string RelativeToRoot(string root, string? path)
    {
        if (string.IsNullOrWhiteSpace(path))
        {
            return "";
        }

        var relative = Path.GetRelativePath(root, path).Replace('\\', '/');
        if (relative == ".."
            || relative.StartsWith("../", StringComparison.Ordinal)
            || Path.IsPathRooted(relative))
        {
            return "";
        }

        return relative;
    }
}
