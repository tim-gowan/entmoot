package io.entmoot.java;

import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.ExpressionTree;
import com.sun.source.tree.IdentifierTree;
import com.sun.source.tree.MemberSelectTree;
import com.sun.source.tree.MethodInvocationTree;
import com.sun.source.tree.MethodTree;
import com.sun.source.tree.NewClassTree;
import com.sun.source.tree.Tree;
import com.sun.source.util.JavacTask;
import com.sun.source.util.SourcePositions;
import com.sun.source.util.TreePath;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.Trees;
import javax.lang.model.element.Element;
import javax.lang.model.element.ElementKind;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.TypeElement;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.StandardLocation;
import javax.tools.ToolProvider;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * Explicit JDK compiler Trees tier — shipped Java materialization (ADR-0012).
 * Never silently replaces another engine selection.
 */
final class JavacTreesCollector {
    private JavacTreesCollector() {}

    static void collect(Path root, List<Path> sources, Main.Result result, boolean canon) throws IOException {
        root = JavaInputs.resolved(root);
        JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
        if (compiler == null) {
            result.invalidEnvironment = true;
            result.diagnostics.add("JDK compiler tools unavailable (install a full JDK, not a JRE)");
            return;
        }

        DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
        JavaInputs inputs = new JavaInputs(root, sources, result);
        try (StandardJavaFileManager fileManager =
                     compiler.getStandardFileManager(diagnostics, Locale.ROOT, StandardCharsets.UTF_8)) {
            fileManager.setLocationFromPaths(StandardLocation.CLASS_PATH, inputs.classpath);
            fileManager.setLocationFromPaths(StandardLocation.SOURCE_PATH, List.of());
            Iterable<? extends JavaFileObject> compilationUnits =
                    fileManager.getJavaFileObjectsFromPaths(inputs.sources);
            List<String> options = new ArrayList<>();
            options.add("-proc:none");
            options.add("-Xlint:none");
            options.add("--release"); options.add(inputs.release);
            options.add("-implicit:none");
            result.buildContext.put("compiler_options", options);

            JavacTask task = (JavacTask) compiler.getTask(
                    null, fileManager, diagnostics, options, null, compilationUnits);
            Iterable<? extends CompilationUnitTree> units = task.parse();
            try {
                task.analyze();
            } catch (Throwable analyzeError) {
                result.diagnostics.add("javac analyze partial: " + analyzeError.getMessage());
                result.invalidEnvironment = true;
            }

            List<Map<String,Object>> canonExpressions = new ArrayList<>();
            if (canon) result.buildContext.put("canon_expressions", canonExpressions);
            final boolean captureScalars = canon && diagnostics.getDiagnostics().stream().noneMatch(d -> d.getKind() == javax.tools.Diagnostic.Kind.ERROR);
            Trees trees = Trees.instance(task);
            SourcePositions positions = trees.getSourcePositions();
            Map<Path, Main.Utf8OffsetMap> offsetMaps = new HashMap<>();

            for (CompilationUnitTree unit : units) {
                JavaFileObject sourceFile = unit.getSourceFile();
                if (sourceFile == null) {
                    continue;
                }
                Path absolute = JavaInputs.retained(Path.of(sourceFile.toUri()));
                String relative = Main.relativePosix(root, absolute);
                if (relative.isEmpty()) {
                    continue;
                }
                Main.Utf8OffsetMap map = offsetMaps.computeIfAbsent(absolute, path -> {
                    try {
                        return Main.Utf8OffsetMap.fromUtf8(java.nio.file.Files.readAllBytes(path));
                    } catch (IOException ex) {
                        return Main.Utf8OffsetMap.empty();
                    }
                });

                new TreePathScanner<Void, Void>() {
                    @Override
                    public Void scan(Tree node, Void unused) {
                        if (captureScalars && node != null && getCurrentPath() != null && canonExpressions.size() < 100000) {
                            TreePath current = new TreePath(getCurrentPath(), node);
                            var type = trees.getTypeMirror(current);
                            Map<String,Object> sort = null;
                            if (type != null) {
                                int bits;
                                switch (type.getKind()) { case BYTE: bits = 8; break; case SHORT: case CHAR: bits = 16; break; case INT: bits = 32; break; case LONG: bits = 64; break; default: bits = 0; }
                                if (type.getKind() == javax.lang.model.type.TypeKind.BOOLEAN) sort = Map.of("kind", "bool");
                                else if (bits > 0) sort = Map.of("kind", "integer", "bits", bits, "signed", type.getKind() != javax.lang.model.type.TypeKind.CHAR);
                            }
                            long start = positions.getStartPosition(unit, node), end = positions.getEndPosition(unit, node);
                            if (sort != null && start >= 0 && end > start) {
                                canonExpressions.add(Map.of("path", relative, "start_byte", map.byteAtClamp((int)start), "end_byte", map.byteAtClamp((int)end), "sort", sort, "overflow", "wrap"));
                            }
                        }
                        return super.scan(node, unused);
                    }
                    @Override
                    public Void visitClass(com.sun.source.tree.ClassTree node, Void unused) {
                        export(getCurrentPath());
                        return super.visitClass(node, unused);
                    }

                    @Override
                    public Void visitMethod(MethodTree node, Void unused) {
                        export(getCurrentPath());
                        return super.visitMethod(node, unused);
                    }

                    private void export(TreePath path) {
                        Element element = trees.getElement(path);
                        if (!(element instanceof TypeElement) && !(element instanceof ExecutableElement)) return;
                        // Compiler-generated constructors/members have no source
                        // declaration and cannot invent structural addresses.
                        if (positions.getEndPosition(unit, path.getLeaf()) < 0) return;
                        Main.CallTarget target = new Main.CallTarget();
                        target.kind = element instanceof TypeElement ? "type" : "callable";
                        fillDeclarationSpan(element, trees, root, offsetMaps, target);
                        inputs.export(element, absolute, target);
                    }
                    @Override
                    public Void visitMethodInvocation(MethodInvocationTree node, Void unused) {
                        emitCall(getCurrentPath(), node, node.getMethodSelect());
                        return super.visitMethodInvocation(node, unused);
                    }

                    @Override
                    public Void visitNewClass(NewClassTree node, Void unused) {
                        emitCall(getCurrentPath(), node, node.getIdentifier());
                        return super.visitNewClass(node, unused);
                    }

                    private void emitCall(TreePath path, Tree node, ExpressionTree select) {
                        long startChar = positions.getStartPosition(unit, node);
                        long endChar = positions.getEndPosition(unit, node);
                        if (startChar < 0 || endChar < 0) {
                            return;
                        }
                        Main.CallFact call = new Main.CallFact();
                        call.path = relative;
                        call.startByte = map.byteAtClamp((int) startChar);
                        call.endByte = map.byteAtClamp((int) endChar);
                        call.owner = findOwner(path, unit, positions, map, root, offsetMaps);

                        Element element = trees.getElement(path);
                        if (element == null && select != null) {
                            element = trees.getElement(TreePath.getPath(path, select));
                        }

                        if (isFunctionalIndirect(select, element)) {
                            call.gap = "indirect_call";
                            result.calls.add(call);
                            return;
                        }

                        if (!(element instanceof ExecutableElement)) {
                            // Missing or non-executable binding is not runtime indirection.
                            call.gap = "unresolved";
                            result.calls.add(call);
                            return;
                        }

                        ExecutableElement method = (ExecutableElement) element;
                        Main.CallTarget target = new Main.CallTarget();
                        target.symbol = ((TypeElement) method.getEnclosingElement()).getQualifiedName() + "#" + method;
                        target.kind = "callable";
                        fillDeclarationSpan(method, trees, root, offsetMaps, target);
                        call.targets.add(target);

                        Element enclosing = method.getEnclosingElement();
                        boolean virtual = !method.getModifiers().contains(javax.lang.model.element.Modifier.STATIC)
                            && !method.getModifiers().contains(javax.lang.model.element.Modifier.PRIVATE)
                            && !method.getModifiers().contains(javax.lang.model.element.Modifier.FINAL)
                            && enclosing != null && !enclosing.getModifiers().contains(javax.lang.model.element.Modifier.FINAL)
                            && method.getKind() == ElementKind.METHOD;
                        boolean superCall = select instanceof com.sun.source.tree.MemberSelectTree
                            && ((com.sun.source.tree.MemberSelectTree) select).getExpression().toString().equals("super");
                        if (virtual && !superCall) {
                            call.gap = "dynamic_dispatch";
                        } else if (call.gap == null && call.owner != null && call.targets.size() == 1
                            && "callable".equals(target.kind)) {
                            call.dispatch = closedDispatch(method, superCall);
                        }
                        try { inputs.reference(method, call, trees, fileManager); }
                        catch (IOException error) { throw new IllegalArgumentException(error); }
                        result.calls.add(call);
                    }
                }.scan(unit, null);
            }
        }

        diagnostics.getDiagnostics().forEach(d -> {
            if (d.getKind() == javax.tools.Diagnostic.Kind.ERROR) {
                result.diagnostics.add(d.toString());
                result.invalidEnvironment = true;
            }
        });
        inputs.verify();
        result.buildContext.put("package_api", inputs.api());
    }

    private static Map<String, Object> closedDispatch(ExecutableElement method, boolean superCall) {
        String policy;
        String fence;
        if (superCall) {
            policy = "entmoot.java.direct-super.v1";
            fence = "source-level super call with non-virtual dispatch; foreign and interface replacement are outside this qualification";
        } else if (method.getModifiers().contains(javax.lang.model.element.Modifier.STATIC)) {
            policy = "entmoot.java.direct-static.v1";
            fence = "source-level static method invocation; foreign and interface replacement are outside this qualification";
        } else if (method.getModifiers().contains(javax.lang.model.element.Modifier.PRIVATE)) {
            policy = "entmoot.java.direct-private.v1";
            fence = "source-level private method invocation; foreign and interface replacement are outside this qualification";
        } else if (method.getModifiers().contains(javax.lang.model.element.Modifier.FINAL)
            || (method.getEnclosingElement() != null
                && method.getEnclosingElement().getModifiers().contains(javax.lang.model.element.Modifier.FINAL))) {
            policy = "entmoot.java.direct-final.v1";
            fence = "source-level final method or final-type instance call; foreign and open virtual replacement are outside this qualification";
        } else {
            policy = "entmoot.java.direct-instance.v1";
            fence = "source-level non-virtual instance method; foreign, virtual, and interface replacement are outside this qualification";
        }
        Map<String, Object> dispatch = new HashMap<>();
        dispatch.put("kind", "closed");
        dispatch.put("policy_id", policy);
        dispatch.put("evidence_pointer", "/calls");
        dispatch.put("assumptions", List.of(
            "selected javac --release, source roots and classpath",
            fence));
        return dispatch;
    }

    private static boolean isFunctionalIndirect(ExpressionTree select, Element element) {
        if (element instanceof ExecutableElement) {
            ExecutableElement method = (ExecutableElement) element;
            Element enclosing = method.getEnclosingElement();
            if (enclosing instanceof TypeElement) {
                String qn = ((TypeElement) enclosing).getQualifiedName().toString();
                if (qn.startsWith("java.util.function.") || "java.lang.Runnable".equals(qn)) {
                    return true;
                }
            }
        }
        if (select instanceof MemberSelectTree) {
            ExpressionTree expr = ((MemberSelectTree) select).getExpression();
            if (expr instanceof IdentifierTree) {
                // Heuristic: named functional parameter style callbacks.
                String name = ((IdentifierTree) expr).getName().toString();
                if ("callback".equals(name) || "fn".equals(name) || "func".equals(name)) {
                    return true;
                }
            }
        }
        return false;
    }

    private static void fillDeclarationSpan(
            Element method,
            Trees trees,
            Path root,
            Map<Path, Main.Utf8OffsetMap> offsetMaps,
            Main.CallTarget target) {
        try {
            TreePath declPath = trees.getPath(method);
            if (method instanceof ExecutableElement && method.getKind() == ElementKind.CONSTRUCTOR
                    && declPath != null && trees.getSourcePositions().getEndPosition(declPath.getCompilationUnit(), declPath.getLeaf()) < 0) {
                declPath = trees.getPath(method.getEnclosingElement());
                target.kind = "type";
            }
            if (declPath == null) {
                target.startByte = 0;
                target.endByte = 0;
                return;
            }
            CompilationUnitTree unit = declPath.getCompilationUnit();
            // A source signature is evidence of a declaration, not a selected
            // implementation. Default interface methods with bodies stay callable.
            if (declPath.getLeaf() instanceof MethodTree
                    && ((MethodTree) declPath.getLeaf()).getBody() == null) {
                target.kind = "callable_signature";
            }
            JavaFileObject sourceFile = unit.getSourceFile();
            if (sourceFile == null) {
                target.startByte = 0;
                target.endByte = 0;
                return;
            }
            Path absolute = JavaInputs.retained(Path.of(sourceFile.toUri()));
            target.originFile = absolute.toString();
            String relative = Main.relativePosix(root, absolute);
            if (!relative.isEmpty()) {
                target.path = relative;
            }
            SourcePositions positions = trees.getSourcePositions();
            long startChar = positions.getStartPosition(unit, declPath.getLeaf());
            long endChar = positions.getEndPosition(unit, declPath.getLeaf());
            Main.Utf8OffsetMap map = offsetMaps.computeIfAbsent(absolute, path -> {
                try {
                    return Main.Utf8OffsetMap.fromUtf8(java.nio.file.Files.readAllBytes(path));
                } catch (IOException ex) {
                    return Main.Utf8OffsetMap.empty();
                }
            });
            if (startChar >= 0 && endChar >= 0) {
                target.startByte = map.byteAtClamp((int) startChar);
                target.endByte = map.byteAtClamp((int) endChar);
            }
        } catch (Throwable ignored) {
            target.startByte = 0;
            target.endByte = 0;
        }
    }

    private static Main.SpanRef findOwner(
            TreePath path,
            CompilationUnitTree unit,
            SourcePositions positions,
            Main.Utf8OffsetMap map,
            Path root,
            Map<Path, Main.Utf8OffsetMap> offsetMaps) {
        TreePath cursor = path;
        while (cursor != null) {
            Tree leaf = cursor.getLeaf();
            if (leaf instanceof MethodTree) {
                long startChar = positions.getStartPosition(unit, leaf);
                long endChar = positions.getEndPosition(unit, leaf);
                if (startChar < 0 || endChar < 0) {
                    return null;
                }
                Main.SpanRef owner = new Main.SpanRef();
                JavaFileObject sourceFile = unit.getSourceFile();
                if (sourceFile == null) {
                    return null;
                }
                Path absolute = JavaInputs.retained(Path.of(sourceFile.toUri()));
                String relative = Main.relativePosix(root, absolute);
                if (relative.isEmpty()) {
                    return null;
                }
                owner.path = relative;
                owner.startByte = map.byteAtClamp((int) startChar);
                owner.endByte = map.byteAtClamp((int) endChar);
                return owner;
            }
            cursor = cursor.getParentPath();
        }
        return null;
    }
}
