using System.Text;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.Text;

namespace Entmoot.RoslynSolver;

/// <summary>
/// Maps Roslyn UTF-16 TextSpan positions to UTF-8 byte offsets for a syntax tree.
/// </summary>
internal sealed class Utf8OffsetMap
{
    private readonly int[] _charToByte;

    private Utf8OffsetMap(int[] charToByte)
    {
        _charToByte = charToByte;
    }

    public static Utf8OffsetMap FromText(SourceText text, string path)
    {
        var source = text.ToString();
        var map = new int[source.Length + 1];
        // Roslyn removes the UTF-8 preamble; inventory spans address file bytes.
        using (var stream = File.OpenRead(path))
        {
            Span<byte> prefix = stackalloc byte[3];
            if (stream.Read(prefix) == 3 && prefix[0] == 0xef && prefix[1] == 0xbb && prefix[2] == 0xbf)
                map[0] = 3;
        }
        for (var i = 0; i < source.Length; i++)
        {
            if (char.IsHighSurrogate(source[i])
                && i + 1 < source.Length
                && char.IsLowSurrogate(source[i + 1]))
            {
                // Surrogate pair encodes as a single UTF-8 scalar (often 4 bytes).
                map[i + 1] = map[i];
                map[i + 2] = map[i] + Encoding.UTF8.GetByteCount(source.AsSpan(i, 2));
                i++;
            }
            else
            {
                map[i + 1] = map[i] + Encoding.UTF8.GetByteCount(source.AsSpan(i, 1));
            }
        }

        return new Utf8OffsetMap(map);
    }

    public int ToByte(int charOffset)
    {
        if (charOffset < 0)
        {
            return 0;
        }

        if (charOffset >= _charToByte.Length)
        {
            return _charToByte[^1];
        }

        return _charToByte[charOffset];
    }

    public (int Start, int End) ToBytes(TextSpan span) =>
        (ToByte(span.Start), ToByte(span.End));
}
