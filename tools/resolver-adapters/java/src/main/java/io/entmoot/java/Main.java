package io.entmoot.java;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.annotations.SerializedName;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.stream.Collectors;
import java.util.stream.Stream;

/**
 * Entmoot Java native solver sidecar (javac Trees only): stdin JSON → stdout JSON.
 * Retired Spoon CTModel override lives only under local {@code .tmp/retired/}.
 */
public final class Main {
    private static final Gson GSON = new GsonBuilder().disableHtmlEscaping().create();

    private Main() {}

    public static void main(String[] args) {
        Result result = new Result();
        try {
            String payload = readStdin();
            Request request = GSON.fromJson(payload, Request.class);
            if (request == null || request.root == null || request.root.isBlank()) {
                result.invalidEnvironment = true;
                result.diagnostics.add("root is required");
                write(result);
                return;
            }

            Path root;
            try {
                root = Paths.get(request.root).toAbsolutePath().normalize();
            } catch (Exception ex) {
                result.invalidEnvironment = true;
                result.diagnostics.add("invalid root path: " + ex.getMessage());
                write(result);
                return;
            }

            if (!Files.isDirectory(root)) {
                result.invalidEnvironment = true;
                result.diagnostics.add("root directory does not exist: " + root);
                write(result);
                return;
            }

            String retiredEngine = System.getenv("ENTMOOT_JAVA_ENGINE");
            if (retiredEngine != null && !retiredEngine.isBlank()) {
                result.invalidEnvironment = true;
                result.diagnostics.add(
                        "unsupported Java engine selection (javac-only; ENTMOOT_JAVA_ENGINE is retired): "
                                + retiredEngine);
                write(result);
                return;
            }

            result.buildContext.put("project_system", "javac_trees");
            result.buildContext.put("java_engine", "javac");
            result.compilerVersion = System.getProperty("java.version", "unknown");

            Set<String> inputs = new LinkedHashSet<>();
            List<Path> sources = discoverJavaSources(root, request.patterns, inputs, result.diagnostics);
            result.inputs.addAll(inputs);
            Collections.sort(result.inputs);

            if (sources.isEmpty() && System.getenv("ENTMOOT_JAVA_ENVIRONMENT") == null) {
                result.diagnostics.add("no Java sources discovered under root");
                write(result);
                return;
            }

            if (request.plan) {
                write(result);
                return;
            }

            try {
                JavacTreesCollector.collect(root, sources, result, request.canon);
            } catch (NoClassDefFoundError | ExceptionInInitializerError missing) {
                result.diagnostics.add("javac unavailable: " + missing);
                result.invalidEnvironment = true;
            } catch (Throwable failure) {
                result.diagnostics.add("javac analysis failed: " + failure.getMessage());
                result.invalidEnvironment = true;
            }

            result.calls.sort(Comparator
                    .comparing((CallFact c) -> c.path)
                    .thenComparingInt(c -> c.startByte));
            Collections.sort(result.diagnostics);
            write(result);
        } catch (Throwable fatal) {
            result.invalidEnvironment = true;
            result.diagnostics.add("fatal: " + fatal.getMessage());
            result.compilerVersion = System.getProperty("java.version", "unknown");
            write(result);
        }
    }

    private static List<Path> discoverJavaSources(
            Path root, List<String> patterns, Set<String> inputs, List<String> diagnostics) {
        List<Path> sources = new ArrayList<>();
        try {
            if (patterns == null || patterns.isEmpty()) {
                try (Stream<Path> walk = sourceTree(root).stream()) {
                    walk.filter(Files::isRegularFile)
                            .filter(p -> p.getFileName().toString().endsWith(".java"))
                            .filter(p -> !isIgnoredSource(root, p))
                            .sorted()
                            .forEach(p -> {
                                sources.add(p);
                                String rel = relativePosix(root, p);
                                if (!rel.isEmpty()) {
                                    inputs.add(rel);
                                }
                            });
                }
            } else {
                for (String pattern : patterns) {
                    Path candidate = root.resolve(pattern).normalize();
                    if (Files.isRegularFile(candidate) && candidate.toString().endsWith(".java")) {
                        sources.add(candidate);
                        String rel = relativePosix(root, candidate);
                        if (!rel.isEmpty()) {
                            inputs.add(rel);
                        }
                    } else if (Files.isDirectory(candidate)) {
                        try (Stream<Path> walk = sourceTree(candidate).stream()) {
                            walk.filter(Files::isRegularFile)
                                    .filter(p -> p.getFileName().toString().endsWith(".java"))
                                    .filter(p -> !isIgnoredSource(root, p))
                                    .sorted()
                                    .forEach(p -> {
                                        sources.add(p);
                                        String rel = relativePosix(root, p);
                                        if (!rel.isEmpty()) {
                                            inputs.add(rel);
                                        }
                                    });
                        }
                    } else {
                        diagnostics.add("pattern did not match a Java file or directory: " + pattern);
                    }
                }
            }
        } catch (IOException ex) {
            diagnostics.add("source discovery failed: " + ex.getMessage());
        }
        return sources.stream().distinct().collect(Collectors.toList());
    }

    /** Prune before traversal; old scratch projects never enter a compiler universe. */
    static List<Path> sourceTree(Path root) throws IOException {
        List<Path> entries = new ArrayList<>();
        Files.walkFileTree(root, new java.nio.file.SimpleFileVisitor<Path>() {
            @Override public java.nio.file.FileVisitResult preVisitDirectory(Path directory, java.nio.file.attribute.BasicFileAttributes attributes) {
                if (!directory.equals(root) && isIgnoredSource(root, directory))
                    return java.nio.file.FileVisitResult.SKIP_SUBTREE;
                entries.add(directory);
                return java.nio.file.FileVisitResult.CONTINUE;
            }
            @Override public java.nio.file.FileVisitResult visitFile(Path file, java.nio.file.attribute.BasicFileAttributes attributes) throws IOException {
                if (isIgnoredSource(root, file)) return java.nio.file.FileVisitResult.CONTINUE;
                if (attributes.isSymbolicLink()) throw new IOException("linked Java source input: " + file);
                if (attributes.isRegularFile() && file.toString().endsWith(".java")) entries.add(file);
                return java.nio.file.FileVisitResult.CONTINUE;
            }
        });
        Collections.sort(entries);
        return entries;
    }

    private static boolean isIgnoredSource(Path root, Path path) {
        Path relative;
        try {
            relative = root.relativize(path);
        } catch (IllegalArgumentException ex) {
            return true;
        }
        for (Path part : relative) {
            String name = part.toString();
            if ((name.startsWith(".") && !name.equals(".github"))
                    || Set.of(
                                    "target",
                                    "build",
                                    "dist",
                                    "node_modules",
                                    "venv",
                                    "__pycache__",
                                    "coverage",
                                    "htmlcov",
                                    // Guava-style alternate universes collide with the main jar sources.
                                    "guava-gwt",
                                    "guava-testlib",
                                    "guava-tests",
                                    "src-super",
                                    "test-super",
                                    "futures",
                                    "android",
                                    "integration-tests",
                                    "benchmark",
                                    "refactorings")
                            .contains(name)
                    || name.endsWith("-gwt")) {
                return true;
            }
        }
        return false;
    }

    static String relativePosix(Path root, Path absolute) {
        try {
            Path base = JavaInputs.retained(root);
            Path file = JavaInputs.retained(absolute);
            Path relative = base.relativize(file);
            String text = relative.toString().replace('\\', '/');
            if (text.equals("..") || text.startsWith("../") || Paths.get(text).isAbsolute()) {
                return "";
            }
            return text;
        } catch (IllegalArgumentException ex) {
            return "";
        }
    }

    private static String readStdin() throws IOException {
        StringBuilder sb = new StringBuilder();
        try (BufferedReader reader =
                     new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8))) {
            String line;
            while ((line = reader.readLine()) != null) {
                sb.append(line).append('\n');
            }
        }
        return sb.toString();
    }

    private static void write(Result result) {
        if (result.calls == null) {
            result.calls = new ArrayList<>();
        }
        if (result.inputs == null) {
            result.inputs = new ArrayList<>();
        }
        if (result.diagnostics == null) {
            result.diagnostics = new ArrayList<>();
        }
        if (result.buildContext == null) {
            result.buildContext = new LinkedHashMap<>();
        }
        System.out.print(GSON.toJson(result));
        System.out.print('\n');
    }

    static final class Request {
        String root;
        List<String> patterns = new ArrayList<>();
        List<String> flags = new ArrayList<>();
        boolean plan;
        boolean canon;
    }

    static final class Result {
        @SerializedName("build_context")
        Map<String, Object> buildContext = new LinkedHashMap<>();
        @SerializedName("compiler_version")
        String compilerVersion = "";
        List<CallFact> calls = new ArrayList<>();
        List<String> inputs = new ArrayList<>();
        List<String> diagnostics = new ArrayList<>();
        @SerializedName("invalid_environment")
        boolean invalidEnvironment;
    }

    static final class CallFact {
        String path = "";
        @SerializedName("start_byte")
        int startByte;
        @SerializedName("end_byte")
        int endByte;
        List<CallTarget> targets = new ArrayList<>();
        SpanRef owner;
        String gap;
        Map<String, Object> dispatch;
        boolean intrinsic;
    }

    static final class CallTarget {
        @SerializedName("origin_file")
        String originFile;
        @SerializedName("package_purl")
        String packagePurl;
        @SerializedName("package_role")
        String packageRole;
        String path;
        @SerializedName("start_byte")
        int startByte;
        @SerializedName("end_byte")
        int endByte;
        String symbol = "";
        String kind = "callable";
    }

    static final class SpanRef {
        String path = "";
        @SerializedName("start_byte")
        int startByte;
        @SerializedName("end_byte")
        int endByte;
    }

    /** Maps Java UTF-16 char offsets onto UTF-8 byte offsets. */
    static final class Utf8OffsetMap {
        private final int[] charToByte;
        private final String text;

        private Utf8OffsetMap(String text, int[] charToByte) {
            this.text = text;
            this.charToByte = charToByte;
        }

        static Utf8OffsetMap empty() {
            return new Utf8OffsetMap("", new int[] {0});
        }

        static Utf8OffsetMap fromUtf8(byte[] utf8) {
            String text = new String(utf8, StandardCharsets.UTF_8);
            int[] map = new int[text.length() + 1];
            int byteIndex = 0;
            for (int i = 0; i < text.length(); i++) {
                map[i] = byteIndex;
                char ch = text.charAt(i);
                if (Character.isHighSurrogate(ch) && i + 1 < text.length()
                        && Character.isLowSurrogate(text.charAt(i + 1))) {
                    byteIndex += new String(new char[] {ch, text.charAt(i + 1)})
                            .getBytes(StandardCharsets.UTF_8).length;
                    map[i + 1] = map[i];
                    i++;
                } else {
                    byteIndex += String.valueOf(ch).getBytes(StandardCharsets.UTF_8).length;
                }
            }
            map[text.length()] = byteIndex;
            return new Utf8OffsetMap(text, map);
        }

        int byteAtClamp(int charIndex) {
            if (charIndex < 0) {
                return 0;
            }
            if (charIndex >= charToByte.length) {
                return charToByte[charToByte.length - 1];
            }
            return charToByte[charIndex];
        }
    }
}
