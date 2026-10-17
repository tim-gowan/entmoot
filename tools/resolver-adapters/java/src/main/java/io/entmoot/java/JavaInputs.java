package io.entmoot.java;

import com.google.gson.*;
import java.nio.file.*;
import java.io.*;
import java.net.*;
import java.util.*;
import javax.lang.model.element.*;
import javax.tools.*;
import com.sun.source.util.Trees;
import org.bouncycastle.crypto.digests.Blake3Digest;

/** Explicit compiler input materialization. Never invokes Maven, Gradle or processors. */
final class JavaInputs {
    final List<Path> sources = new ArrayList<>();
    final List<Path> classpath = new ArrayList<>();
    final Map<String, String> digests = new TreeMap<>();
    final Map<String, List<String>> directories = new TreeMap<>();
    final Map<Path, String> sourcePackages = new TreeMap<>();
    final Map<Path, String> binaryPackages = new TreeMap<>();
    final Map<String, Map<String, Object>> exports = new TreeMap<>();
    final List<Object> references = new ArrayList<>();
    final Main.Result result;
    String release;
    static final String METHOD = "javac.public_source_method_contract.v1";
    static final String BINARY = "javac.public_binary_method_contract.v1";
    static final String TYPE = "javac.public_source_type_contract.v1";

    /**
     * Prefer real paths when the host allows them. Some Windows TEMP roots deny
     * {@link Path#toRealPath()}; absolute normalize still fences the retained input.
     * Real paths also collapse 8.3 TEMP aliases so package-root prefix checks match.
     */
    static Path resolved(Path path) throws IOException {
        try {
            return path.toRealPath();
        } catch (java.nio.file.AccessDeniedException denied) {
            return path.toAbsolutePath().normalize();
        }
    }

    /** Best-effort retained path for compiler-reported URIs (never throws). */
    static Path retained(Path path) {
        try {
            return resolved(path);
        } catch (IOException error) {
            return path.toAbsolutePath().normalize();
        }
    }

    JavaInputs(Path root, List<Path> discovered, Main.Result result) throws IOException {
        this.result = result;
        sources.addAll(discovered);
        release = System.getenv().getOrDefault("ENTMOOT_JAVA_RELEASE", "17");
        String environment = System.getenv("ENTMOOT_JAVA_ENVIRONMENT");
        if (environment != null) {
            Path config = resolved(Path.of(environment));
            remember(config);
            JsonObject profile = JsonParser.parseString(Files.readString(config)).getAsJsonObject();
            for (String key : profile.keySet())
                if (!List.of("release", "source_roots", "classpath").contains(key))
                    throw new IllegalArgumentException("unknown Java environment field: " + key);
            if (profile.has("release")) release = profile.get("release").getAsString();
            // Explicit source_roots replace host discovery (tests/fixtures stay out).
            if (profile.has("source_roots")) {
                sources.clear();
                for (JsonElement row : profile.getAsJsonArray("source_roots")) {
                    Path directory = resolved(config.getParent().resolve(row.getAsString()));
                    try (var paths = Main.sourceTree(directory).stream()) {
                        for (Path file : paths.sorted().collect(java.util.stream.Collectors.toList())) {
                            if (Files.isSymbolicLink(file)) throw new IllegalArgumentException("linked Java source input");
                            if (Files.isDirectory(file)) rememberDirectory(file);
                            else if (file.toString().endsWith(".java")) sources.add(file);
                        }
                    }
                }
            }
            if (profile.has("classpath")) for (JsonElement row : profile.getAsJsonArray("classpath"))
                classpath.add(resolved(config.getParent().resolve(row.getAsString())));
            result.buildContext.put("compiler_environment", profile);
        } else {
            String configured = System.getenv("ENTMOOT_JAVA_CLASSPATH");
            if (configured != null && !configured.isBlank()) for (String file : configured.split(java.util.regex.Pattern.quote(File.pathSeparator)))
                classpath.add(resolved(Path.of(file)));
        }
        if (!List.of("8", "11", "17", "21", "25").contains(release))
            throw new IllegalArgumentException("unqualified javac release: " + release);
        // Keep real paths so package-root startsWith matches (Windows 8.3 TEMP vs long path).
        List<Path> retained = new ArrayList<>();
        for (Path file : sources) retained.add(resolved(file));
        TreeSet<Path> distinct = new TreeSet<>(retained);
        sources.clear(); sources.addAll(distinct);
        for (Path file : sources) { remember(file); rememberDirectory(file.getParent()); }
        for (Path file : classpath) {
            if (!Files.isRegularFile(file) || !file.toString().endsWith(".jar"))
                throw new IllegalArgumentException("Java classpath requires retained jars: " + file);
            remember(file);
        }
        // --release reads the JDK symbol archive; the actual native compiler
        // implementation and platform image are retained inputs as well.
        Path home = Path.of(System.getProperty("java.home"));
        for (String file : List.of("release", "lib/modules", "lib/ct.sym")) {
            Path input = home.resolve(file);
            if (Files.isRegularFile(input)) remember(input);
        }
        try {
            Path adapter = Path.of(JavaInputs.class.getProtectionDomain().getCodeSource().getLocation().toURI());
            if (Files.isRegularFile(adapter)) remember(adapter);
        } catch (URISyntaxException error) { throw new IOException(error); }
        String mapping = System.getenv("ENTMOOT_PACKAGE_API");
        if (mapping != null) {
            Path file = resolved(Path.of(mapping)); remember(file);
            JsonObject document = JsonParser.parseString(Files.readString(file)).getAsJsonObject();
            for (JsonElement row : document.getAsJsonArray("packages")) {
                JsonObject entry = row.getAsJsonObject();
                if (!entry.get("ecosystem").getAsString().equals("java")) continue;
                String name = entry.get("name").getAsString();
                if (!name.matches("[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+")) throw new IllegalArgumentException("invalid Maven coordinate");
                if (entry.has("root") == entry.has("jar")) throw new IllegalArgumentException("Java mapping needs exactly one root or jar");
                Map<Path,String> mappings = entry.has("root") ? sourcePackages : binaryPackages;
                Path location = resolved(Path.of(entry.get(entry.has("root") ? "root" : "jar").getAsString()));
                if (mappings.putIfAbsent(location, name) != null) throw new IllegalArgumentException("duplicate Java package mapping");
            }
            result.buildContext.put("package_mapping", document);
        }
    }

    static String hash(Path path) throws IOException {
        Blake3Digest digest = new Blake3Digest();
        byte[] block = new byte[65536], output = new byte[32];
        try (InputStream stream = Files.newInputStream(path)) {
            int count; while ((count = stream.read(block)) != -1) digest.update(block, 0, count);
        }
        digest.doFinal(output, 0);
        StringBuilder text = new StringBuilder("blake3:");
        for (byte value : output) text.append(String.format("%02x", value & 255));
        return text.toString();
    }
    void remember(Path file) throws IOException { digests.put(file.toString(), hash(file)); }
    void rememberDirectory(Path directory) throws IOException {
        try (var entries = Files.list(directory)) {
            directories.put(directory.toString(), entries.map(file -> file.getFileName().toString()).sorted().collect(java.util.stream.Collectors.toList()));
        }
    }
    void verify() throws IOException {
        for (var input : digests.entrySet())
            if (!hash(Path.of(input.getKey())).equals(input.getValue())) throw new IOException("Java input changed during compilation: " + input.getKey());
        for (var directory : new TreeMap<>(directories).entrySet()) {
            rememberDirectory(Path.of(directory.getKey()));
            if (!directory.getValue().equals(directories.get(directory.getKey())))
                throw new IOException("Java source directory changed during compilation: " + directory.getKey());
        }
        result.inputs.addAll(digests.keySet());
        result.buildContext.put("input_digests", digests);
        result.buildContext.put("source_directory_entries", directories);
        result.buildContext.put("source_release", release);
        result.buildContext.put("classpath", classpath.stream().map(Object::toString).collect(java.util.stream.Collectors.toList()));
        result.buildContext.put("annotation_processing", "disabled;generated_sources_require_explicit_materialization");
        result.buildContext.put("input_closure", "explicit_sources_jars_and_jdk_revalidated;no_ambient_sourcepath_or_processors");
    }

    private static boolean visible(Element element) {
        for (Element owner = element; owner != null && !(owner instanceof PackageElement); owner = owner.getEnclosingElement())
            if ((owner instanceof TypeElement || owner instanceof ExecutableElement) && !owner.getModifiers().contains(Modifier.PUBLIC)) return false;
        return true;
    }
    private static String symbol(Element element) {
        if (element instanceof TypeElement) return ((TypeElement) element).getQualifiedName().toString();
        return ((TypeElement) element.getEnclosingElement()).getQualifiedName() + "#" + element;
    }
    private static String contract(Element element) {
        String text = element.asType() + "\n" + element.getModifiers() + "\n" + element.getAnnotationMirrors();
        if (element instanceof ExecutableElement) {
            ExecutableElement method = (ExecutableElement) element;
            text += "\n" + method.getReturnType() + "\n" + method.isVarArgs() + "\n" + method.getThrownTypes() + "\n" + method.getDefaultValue();
            for (TypeParameterElement parameter : method.getTypeParameters()) text += "\n" + parameter + ":" + parameter.getBounds();
            for (VariableElement parameter : method.getParameters()) text += "\n" + parameter + ":" + parameter.asType() + ":" + parameter.getAnnotationMirrors();
            text += "\nowner:" + typeHeader((TypeElement) method.getEnclosingElement());
        } else if (element instanceof TypeElement) {
            TypeElement type = (TypeElement) element;
            text += "\n" + typeHeader(type);
            for (Element member : type.getEnclosedElements()) if (member.getModifiers().contains(Modifier.PUBLIC))
                text += "\nmember:" + member + ":" + (member instanceof ExecutableElement ? contract(member) : member.asType() + ":" + member.getAnnotationMirrors());
        }
        return text;
    }
    private static String typeHeader(TypeElement type) {
        String text = type.getQualifiedName() + "\n" + type.getSuperclass() + "\n" + type.getInterfaces()
            + "\n" + type.getModifiers() + "\n" + type.getAnnotationMirrors();
        for (TypeParameterElement parameter : type.getTypeParameters()) text += "\n" + parameter + ":" + parameter.getBounds();
        return text;
    }
    private String sourcePackage(Path file) {
        Path retained = retained(file);
        List<String> matches = sourcePackages.entrySet().stream().filter(entry -> retained.startsWith(entry.getKey()))
            .map(Map.Entry::getValue).collect(java.util.stream.Collectors.toList());
        return matches.size() == 1 ? matches.get(0) : null;
    }
    void export(Element element, Path source, Main.CallTarget target) {
        if (!visible(element) || target.path == null || target.path.isEmpty()) return;
        String name = sourcePackage(source);
        if (name == null) return;
        String spelling = symbol(element);
        Map<String,Object> row = Map.of("path", target.path, "start_byte", target.startByte, "end_byte", target.endByte,
            "symbol", spelling, "boundary_policy", element instanceof TypeElement ? TYPE : METHOD, "boundary_text", contract(element));
        Object previous = exports.computeIfAbsent(name, ignored -> new TreeMap<>()).putIfAbsent(spelling, row);
        if (previous != null && !previous.equals(row)) throw new IllegalArgumentException("conflicting javac export: " + spelling);
    }
    void reference(ExecutableElement method, Main.CallFact call, Trees trees, StandardJavaFileManager manager) throws IOException {
        if (!visible(method) || call.gap != null || call.targets.size() != 1 || call.targets.get(0).path != null) return;
        String name = null, policy = BINARY;
        var source = trees.getPath(method);
        if (source != null) {
            name = sourcePackage(retained(Path.of(source.getCompilationUnit().getSourceFile().toUri())));
            policy = METHOD;
        } else {
            TypeElement owner = (TypeElement) method.getEnclosingElement();
            JavaFileObject binary = manager.getJavaFileForInput(StandardLocation.CLASS_PATH, owner.getQualifiedName().toString(), JavaFileObject.Kind.CLASS);
            if (binary != null && binary.toUri().getScheme().equals("jar")) {
                JarURLConnection connection = (JarURLConnection) binary.toUri().toURL().openConnection();
                try {
                    Path jar = Path.of(connection.getJarFileURL().toURI()).toRealPath();
                    name = binaryPackages.get(jar);
                    call.targets.get(0).originFile = jar.toString();
                }
                catch (URISyntaxException error) { throw new IOException(error); }
            }
        }
        if (name == null) return;
        String spelling = symbol(method);
        Main.CallTarget target = call.targets.get(0);
        // Keep package coords for seeking; assessed ref still requires contract text.
        target.symbol = spelling; target.packageRole = "module"; target.packagePurl = "pkg:maven/" + name;
        references.add(Map.of("path", call.path, "start_byte", call.startByte, "end_byte", call.endByte,
            "target", Map.of("package", Map.of("ecosystem", "java", "name", name),
                "symbol", Map.of("scheme", "java.compiler.symbol", "spelling", spelling)),
            "boundary_policy", policy, "boundary_text", contract(method)));
    }
    Object api() {
        List<Object> surfaces = new ArrayList<>();
        for (var entry : exports.entrySet()) surfaces.add(Map.of("package", Map.of("ecosystem", "java", "name", entry.getKey()),
            "export_universes", List.of(Map.of("symbol_scheme", "java.compiler.symbol", "boundary_policy", METHOD),
                Map.of("symbol_scheme", "java.compiler.symbol", "boundary_policy", TYPE)),
            "completeness", "partial", "exports", entry.getValue().values(),
            "limitations", List.of("selected_compiler_materialization_only", "runtime_dispatch_unassessed")));
        return Map.of("surfaces", surfaces, "references", references, "references_complete", false,
            "limitations", List.of("source_and_binary_authorities_are_distinct", "typecheck_success_not_compatibility"));
    }
}
