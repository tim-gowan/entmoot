import com.sun.source.util.JavacTask;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.net.URI;
import java.nio.ByteBuffer;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.util.Base64;
import java.util.List;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaFileObject;
import javax.tools.SimpleJavaFileObject;
import javax.tools.ToolProvider;

/** Parse only: no attribution, dependency loading, annotation processing or execution. */
class ParseOracle {
    public static void main(String[] args) throws Exception {
        var compiler = ToolProvider.getSystemJavaCompiler();
        if (compiler == null || Runtime.version().feature() != 27) {
            throw new IllegalStateException("JDK 27 compiler required");
        }
        try (var files = compiler.getStandardFileManager(null, null, StandardCharsets.UTF_8);
             var input = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8))) {
            for (String line; (line = input.readLine()) != null;) {
                String source;
                try {
                    source = StandardCharsets.UTF_8.newDecoder()
                        .onMalformedInput(CodingErrorAction.REPORT)
                        .onUnmappableCharacter(CodingErrorAction.REPORT)
                        .decode(ByteBuffer.wrap(Base64.getDecoder().decode(line))).toString();
                } catch (java.nio.charset.CharacterCodingException error) {
                    System.out.println("unqualified\tencoding-not-utf8");
                    continue;
                }
                var diagnostics = new DiagnosticCollector<JavaFileObject>();
                var file = new SimpleJavaFileObject(URI.create("string:///Fixture.java"), JavaFileObject.Kind.SOURCE) {
                    @Override public CharSequence getCharContent(boolean ignoreEncodingErrors) { return source; }
                };
                var task = (JavacTask) compiler.getTask(null, files, diagnostics,
                    List.of("--source", "27", "--enable-preview", "-proc:none", "-Xlint:none"), null, List.of(file));
                // Iterate to ensure parser diagnostics have actually been produced.
                for (var ignored : task.parse()) { }
                var errors = diagnostics.getDiagnostics().stream()
                    .filter(d -> d.getKind() == javax.tools.Diagnostic.Kind.ERROR)
                    .map(d -> d.getCode() + ":" + d.getLineNumber() + ":" + d.getColumnNumber()).toList();
                System.out.println(errors.isEmpty() ? "accepted\t" : "rejected\t" + String.join(",", errors));
            }
        }
    }
}
