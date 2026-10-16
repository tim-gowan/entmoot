using System.Text.Json;
using Microsoft.CodeAnalysis;

namespace Entmoot.RoslynSolver;

// Assembly-to-package attribution is explicit operator input, retained in raw evidence.
internal sealed class PackageApi
{
    private const string Policy = "roslyn.public_method_source_contract.v2";
    private const string TypePolicy = "roslyn.public_named_type_contract.v1";
    private static string PolicyFor(IMethodSymbol method) => method.DeclaringSyntaxReferences.Length == 0
        ? "roslyn.public_method_metadata_contract.v2" : Policy;
    private readonly Dictionary<string, string> packages = new(StringComparer.Ordinal);
    private readonly Dictionary<string, SortedDictionary<string, object>> exports = new(StringComparer.Ordinal);
    private readonly List<object> references = [];

    private void AddExport(string name, string symbol, object row)
    {
        if (!exports.TryGetValue(name, out var surface)) exports[name] = surface = new(StringComparer.Ordinal);
        if (surface.TryGetValue(symbol, out var previous) && JsonSerializer.Serialize(previous) != JsonSerializer.Serialize(row))
            throw new InvalidOperationException("conflicting project export: " + symbol);
        surface[symbol] = row;
    }

    public static PackageApi Load(Result result)
    {
        var api = new PackageApi();
        var path = Environment.GetEnvironmentVariable("ENTMOOT_PACKAGE_API");
        if (string.IsNullOrEmpty(path)) return api;
        var text = File.ReadAllText(path);
        using var document = JsonDocument.Parse(text);
        foreach (var row in document.RootElement.GetProperty("packages").EnumerateArray())
        {
            if (row.GetProperty("ecosystem").GetString() != "csharp") continue;
            var assembly = row.GetProperty("assembly").GetString() ?? "";
            var name = row.GetProperty("name").GetString()?.ToLowerInvariant() ?? "";
            if (string.IsNullOrWhiteSpace(assembly) || string.IsNullOrWhiteSpace(name) || !api.packages.TryAdd(assembly, name))
                throw new InvalidOperationException("invalid or duplicate assembly package mapping");
        }
        result.Inputs.Add(Path.GetFullPath(path));
        result.BuildContext["package_mapping"] = document.RootElement.Clone();
        return api;
    }

    private string? Package(IMethodSymbol method)
    {
        if (method.MethodKind != MethodKind.Ordinary || method.DeclaredAccessibility != Accessibility.Public) return null;
        for (var type = method.ContainingType; type != null; type = type.ContainingType)
            if (type.DeclaredAccessibility != Accessibility.Public) return null;
        return packages.GetValueOrDefault(method.ContainingAssembly.Identity.Name);
    }

    private static object Coordinate(string name) => new { ecosystem = "csharp", name };
    private static string Constraints(IEnumerable<ITypeParameterSymbol> parameters) => string.Join("\n", parameters.Select(p =>
        p.Name + ":" + p.Variance + ":" + p.HasConstructorConstraint + ":" + p.HasReferenceTypeConstraint + ":"
        + p.ReferenceTypeConstraintNullableAnnotation + ":" + p.HasValueTypeConstraint + ":" + p.HasUnmanagedTypeConstraint + ":" + p.HasNotNullConstraint
        + ":" + string.Join(",", p.ConstraintTypes.Select(t => t.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat)))));
    private static string Attributes(ISymbol symbol) => string.Join("\n", symbol.GetAttributes()
        .Select(a => a.ToString()).OrderBy(a => a, StringComparer.Ordinal));
    private static string Contract(IMethodSymbol method) => method.ReturnType.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat)
        + "\n" + method.IsStatic + ":" + method.IsAbstract + ":" + method.IsVirtual + ":" + method.IsOverride + ":" + method.IsSealed
        + "\n" + Constraints(method.TypeParameters) + "\n" + Constraints(method.ContainingType.TypeParameters)
        + "\n" + Attributes(method) + "\n" + Attributes(method.ContainingType)
        + "\nreturns:" + string.Join("\n", method.GetReturnTypeAttributes().Select(a => a.ToString()).OrderBy(a => a, StringComparer.Ordinal))
        + "\n" + method.ReturnsByRef + ":" + method.ReturnsByRefReadonly
        + "\n" + method.ToDisplayString(SymbolDisplayFormat.CSharpErrorMessageFormat)
        + "\n" + string.Join("\n", method.Parameters.Select(p => p.Name + ":" + p.Type.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat)
            + ":" + p.NullableAnnotation + ":" + Attributes(p) + ":" + p.IsOptional + ":" + p.IsParams + ":" + p.RefKind
            + ":" + JsonSerializer.Serialize(p.HasExplicitDefaultValue ? p.ExplicitDefaultValue : null)));

    public void Export(IMethodSymbol method, CallTarget target)
    {
        method = (method.ReducedFrom ?? method).OriginalDefinition;
        var name = Package(method);
        if (name == null || target.Path == null) return;
        AddExport(name, target.Symbol, new { path = target.Path, start_byte = target.StartByte, end_byte = target.EndByte,
            symbol = target.Symbol, boundary_policy = PolicyFor(method), boundary_text = Contract(method) });
    }

    public void ExportType(INamedTypeSymbol type, CallTarget target)
    {
        if (type.DeclaringSyntaxReferences.Length != 1 || target.Path == null) return;
        for (var owner = type; owner != null; owner = owner.ContainingType)
            if (owner.DeclaredAccessibility != Accessibility.Public) return;
        var name = packages.GetValueOrDefault(type.ContainingAssembly.Identity.Name);
        if (name == null) return;
        var contract = type.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat)
            + "\n" + type.TypeKind + ":" + type.IsAbstract + ":" + type.IsSealed + ":" + type.IsStatic
            + "\n" + Constraints(type.TypeParameters) + "\n" + Attributes(type)
            + "\nbase:" + type.BaseType?.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat)
            + "\ninterfaces:" + string.Join("\n", type.Interfaces.Select(t => t.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat)).OrderBy(t => t, StringComparer.Ordinal))
            + "\nmembers:" + string.Join("\n", type.GetMembers().Where(member => member.DeclaredAccessibility == Accessibility.Public)
                .Select(member => member is IMethodSymbol method ? Contract(method) : member.ToDisplayString(SymbolDisplayFormat.FullyQualifiedFormat)
                    + (member is IFieldSymbol field && field.HasConstantValue ? ":" + JsonSerializer.Serialize(field.ConstantValue) : ""))
                .OrderBy(member => member, StringComparer.Ordinal));
        AddExport(name, target.Symbol, new { path = target.Path, start_byte = target.StartByte, end_byte = target.EndByte,
            symbol = target.Symbol, boundary_policy = TypePolicy, boundary_text = contract });
    }

    public void Reference(IMethodSymbol method, CallFact call)
    {
        method = (method.ReducedFrom ?? method).OriginalDefinition;
        var name = Package(method);
        if (name == null || call.Gap != null || call.Targets.Count != 1 || call.Targets[0].Path != null) return;
        var target = call.Targets[0];
        // Stamp package coords for seeking when mapped but leave assessed emit
        // to explicit boundary text (fill seeks if no assessed ref covers the span).
        target.PackagePurl = "pkg:nuget/" + Uri.EscapeDataString(name);
        target.PackageRole = "module";
        references.Add(new { path = call.Path, start_byte = call.StartByte, end_byte = call.EndByte,
            target = new { package = Coordinate(name), symbol = new { scheme = "csharp.compiler.symbol", spelling = target.Symbol } },
            boundary_policy = PolicyFor(method), boundary_text = Contract(method) });
    }

    public object Result() => new { surfaces = exports.OrderBy(p => p.Key, StringComparer.Ordinal).Select(p => new {
        package = Coordinate(p.Key), export_universes = new[] { new { symbol_scheme = "csharp.compiler.symbol", boundary_policy = Policy },
            new { symbol_scheme = "csharp.compiler.symbol", boundary_policy = TypePolicy } },
        completeness = "partial", exports = p.Value.Values.ToArray(), limitations = new[] { "public_method_declarations_only", "assembly_package_mapping_operator_declared" } }),
        references, references_complete = false, limitations = new[] { "runtime_dispatch_and_type_layout_unassessed", "source_and_binary_contracts_are_distinct" } };
}
