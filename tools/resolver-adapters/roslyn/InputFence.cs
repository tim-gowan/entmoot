using Microsoft.CodeAnalysis;

namespace Entmoot.RoslynSolver;

// Post-evaluation materialization fence. This does not certify arbitrary MSBuild
// target execution or generator negative lookups before the workspace was loaded.
internal sealed class InputFence
{
    public SortedDictionary<string, string> Digests { get; } = new(StringComparer.Ordinal);
    public SortedDictionary<string, string[]> Directories { get; } = new(StringComparer.Ordinal);

    private static string Hash(string file)
    {
        using var stream = File.OpenRead(file);
        using var hasher = Blake3.Hasher.New();
        var buffer = new byte[65536];
        int count;
        while ((count = stream.Read(buffer)) != 0) hasher.Update(buffer.AsSpan(0, count));
        return "blake3:" + hasher.Finalize().ToString();
    }

    private static string[] Entries(string directory) => Directory.EnumerateFileSystemEntries(directory)
        .Select(Path.GetFileName).OfType<string>().OrderBy(name => name, StringComparer.Ordinal).ToArray();

    public InputFence(IEnumerable<string> inputs)
    {
        foreach (var file in inputs.Select(Path.GetFullPath).Distinct(StringComparer.Ordinal).OrderBy(path => path, StringComparer.Ordinal))
        {
            Digests.Add(file, Hash(file));
            var directory = Path.GetDirectoryName(file)!;
            Directories.TryAdd(directory, Entries(directory));
        }
    }

    public void CheckTree(SyntaxTree tree)
    {
        if (string.IsNullOrEmpty(tree.FilePath) || !Digests.ContainsKey(Path.GetFullPath(tree.FilePath)))
            throw new InvalidOperationException("generated source lacks a retained materialization: " + tree.FilePath);
        // Roslyn SourceText and its physical inventory must be the same source.
        if (tree.GetText().ToString() != File.ReadAllText(tree.FilePath))
            throw new InvalidOperationException("compiler source differs from materialization: " + tree.FilePath);
    }

    public void Verify()
    {
        foreach (var input in Digests)
            if (Hash(input.Key) != input.Value) throw new InvalidOperationException("native input changed: " + input.Key);
        foreach (var directory in Directories)
            if (!Entries(directory.Key).SequenceEqual(directory.Value))
                throw new InvalidOperationException("native input directory changed: " + directory.Key);
    }
}
