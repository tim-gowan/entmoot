// Parse only: no attribution, project evaluation, package restore or foreign execution.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;

class ParseOracle
{
    static void Main()
    {
        var options = new CSharpParseOptions(LanguageVersion.CSharp14, DocumentationMode.Parse)
            .WithFeatures(new[] { new KeyValuePair<string, string>("FileBasedProgram", "true") });
        string line;
        while ((line = Console.ReadLine()) != null)
        {
            try
            {
                var source = new UTF8Encoding(false, true).GetString(Convert.FromBase64String(line));
                var tree = CSharpSyntaxTree.ParseText(source, options);
                var errors = tree.GetDiagnostics().Where(d => d.Severity == DiagnosticSeverity.Error)
                    .Select(d => d.Id).Distinct().OrderBy(id => id).ToArray();
                Console.WriteLine((errors.Length == 0 ? "accepted" : "rejected") + "\t" + string.Join(",", errors));
            }
            catch (DecoderFallbackException)
            {
                Console.WriteLine("unqualified\tinvalid_utf8");
            }
        }
    }
}
