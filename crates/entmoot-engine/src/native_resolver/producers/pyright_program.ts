// Entmoot-owned adapter to Pyright 1.1.414; upstream source remains external.
// Preserve direct declarations and native SourceMapper navigation separately.
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { CancellationToken } from 'vscode-languageserver';
import { blake3 } from '@noble/hashes/blake3';
import { bytesToHex } from '@noble/hashes/utils';
import { Program } from '@pyright/analyzer/program';
import { ImportResolver } from '@pyright/analyzer/importResolver';
import { DeclarationType } from '@pyright/analyzer/declaration';
import { getClassFieldsRecursive } from '@pyright/analyzer/typeUtils';
import { ParseTreeWalker } from '@pyright/analyzer/parseTreeWalker';
import { ParseNodeType } from '@pyright/parser/parseNodes';
import { ConfigOptions } from '@pyright/common/configOptions';
import { PythonVersion } from '@pyright/common/pythonVersion';
import { NullConsole } from '@pyright/common/console';
import { FullAccessHost } from '@pyright/common/fullAccessHost';
import { RealTempFile, createFromRealFileSystem } from '@pyright/common/realFileSystem';
import { createServiceProvider } from '@pyright/common/serviceProviderExtensions';
import { Uri } from '@pyright/common/uri/uri';
import { PyrightFileSystem } from '@pyright/pyrightFileSystem';

declare const ENTMOOT_PYRIGHT_TYPESHED: string;
const digest = (bytes: Uint8Array) => 'blake3:' + bytesToHex(blake3(bytes));
const inside = (root: string, file: string) => {
    // Resolve both sides before comparing. Windows short/long path spellings and
    // drive-letter case otherwise make path.relative report a false escape.
    try {
        root = real(root);
        file = real(file);
    } catch {
        return false;
    }
    if (process.platform === 'win32') {
        root = root.replaceAll('/', '\\').replace(/\\+$/, '');
        file = file.replaceAll('/', '\\').replace(/\\+$/, '');
        if (root.toLowerCase() === file.toLowerCase()) return true;
    } else if (root === file) {
        return true;
    }
    const rel = path.relative(root, file);
    return rel === '' || (!rel.startsWith('..' + path.sep) && rel !== '..' && !path.isAbsolute(rel));
};
const portable = (s: string) => s.replaceAll('\\', '/');
const publicMember = (name: string) => !name.startsWith('_') || /^__.+__$/.test(name);
const real = (s: string) => {
    let resolved = fs.realpathSync(s);
    // Sidecar roots are ordinary drive paths; strip verbatim forms so containment
    // matches request.root after Rust compiler_root normalization.
    if (resolved.startsWith('\\\\?\\')) {
        resolved = resolved.slice(4);
        if (resolved.startsWith('UNC\\')) resolved = '\\\\' + resolved.slice(4);
    }
    return resolved.replace(/^[a-z]:/, drive => drive.toUpperCase());
};

/** Match structural-scan skip dirs; keep the Python fence close to inventory. */
const SKIP_DIR_NAMES = new Set([
    '.git', '.entmoot', '.tmp', '.venv', 'venv', 'node_modules', 'target',
    '__pycache__', '.pytest_cache', '.ruff_cache', '.mypy_cache', '.endorlabs',
    '.uv', '.cache', '.eggs', 'dist', 'build', 'htmlcov', '.idea', '.vscode',
    '.semgrep', '.trivycache', 'coverage',
]);

function shouldSkipDirName(name: string): boolean {
    return SKIP_DIR_NAMES.has(name) || (name.startsWith('.') && name !== '.github');
}

function gitRepoRoot(start: string): string | null {
    const result = spawnSync('git', ['-C', start, 'rev-parse', '--show-toplevel'], {
        encoding: 'utf8',
        windowsHide: true,
    });
    if (result.status !== 0) return null;
    const top = result.stdout.trim();
    return top ? real(top) : null;
}

/** Directory/file ignore: scan skip names, then root `.gitignore` via `git check-ignore`. */
function makeIgnoreChecker(fenceRoot: string): (absPath: string) => boolean {
    const repo = gitRepoRoot(fenceRoot);
    // An explicitly selected materialization can live in an enclosing repo's
    // ignored scratch directory. Its own ignore files still apply, but the
    // ancestor must not silently erase the entire selected source universe.
    const detached = repo && spawnSync('git', ['-C', repo, 'check-ignore', '-q', '--', fenceRoot],
        {windowsHide: true}).status === 0;
    const ignoreRoot = detached ? fenceRoot : repo;
    const gitArgs = repo ? ['-C', repo, ...(detached ? ['--work-tree=' + fenceRoot] : [])] : [];
    const cache = new Map<string, boolean>();
    return (absPath: string) => {
        if (shouldSkipDirName(path.basename(absPath))) return true;
        if (!ignoreRoot) return false;
        const rel = path.relative(ignoreRoot, absPath);
        if (!rel || rel.startsWith('..') || path.isAbsolute(rel)) return false;
        const key = rel.replaceAll('\\', '/');
        const cached = cache.get(key);
        if (cached !== undefined) return cached;
        const result = spawnSync('git', [...gitArgs, 'check-ignore', '-q', ...(detached ? ['--no-index'] : []), '--', key], {
            windowsHide: true,
        });
        const ignored = result.status === 0;
        cache.set(key, ignored);
        return ignored;
    };
}

/** Capture the active interpreter's import paths; never invent package coordinates. */
function discoverEnvironment(root: string) {
    const executable = process.platform === 'win32' ? 'python.exe' : 'python';
    const inVenv = (dir: string) => path.join(dir, process.platform === 'win32' ? 'Scripts' : 'bin', executable);
    let interpreter: string | undefined;
    let selection: any;
    for (const variable of ['ENTMOOT_PYTHON_EXECUTABLE', 'VIRTUAL_ENV', 'CONDA_PREFIX']) {
        const value = process.env[variable];
        if (value === undefined) continue;
        if (!value.trim()) throw Error('empty explicit Python environment: ' + variable);
        interpreter = variable === 'ENTMOOT_PYTHON_EXECUTABLE' ? value :
            variable === 'CONDA_PREFIX' && process.platform === 'win32' ? path.join(value, executable) : inVenv(value);
        selection = {kind: 'environment', variable};
        break;
    }
    if (!interpreter) {
        for (const name of ['.venv', 'venv']) {
            const candidate = inVenv(path.join(root, name));
            if (fs.existsSync(candidate)) { interpreter = candidate; selection = {kind: 'project', path: name}; break; }
        }
    }
    if (!interpreter) {
        for (const command of process.platform === 'win32' ? ['python.exe', 'python3.exe'] : ['python3', 'python']) {
            const candidate = (process.env.PATH ?? '').split(path.delimiter).map(dir => path.join(dir, command)).find(file => fs.existsSync(file));
            if (candidate) { interpreter = candidate; selection = {kind: 'path', command}; break; }
        }
    }
    if (!interpreter) throw Error('no installed Python interpreter found');
    interpreter = real(interpreter); // Explicit broken selections never fall through.
    const probe = spawnSync(interpreter, ['-I', '-c', 'import json,sys,site; print(json.dumps({"executable":sys.executable,"paths":site.getsitepackages()}))'],
        {encoding: 'utf8', windowsHide: true, timeout: 5000, maxBuffer: 65536});
    if (probe.status !== 0) throw Error('selected Python interpreter probe failed: ' + String(probe.error ?? probe.stderr));
    const observed = JSON.parse(probe.stdout);
    const roots = [root, path.join(root, 'src'), ...observed.paths].filter(dir => fs.existsSync(dir));
    return {selection, settings: {interpreter: real(observed.executable), import_roots: [...new Set(roots.map(real))], packages: []}};
}

function run(request: any) {
    const root = real(request.root);
    const canonExpressions: any[] = [];
    const environment = process.env.ENTMOOT_PYTHON_ENVIRONMENT;
    if (environment !== undefined && !environment.trim()) throw Error('empty ENTMOOT_PYTHON_ENVIRONMENT');
    const discovered = environment === undefined ? discoverEnvironment(root) : undefined;
    const configBytes = environment !== undefined ? fs.readFileSync(environment) : Buffer.from(JSON.stringify(discovered!.settings));
    const settings = JSON.parse(new TextDecoder('utf-8', {fatal: true}).decode(configBytes));
    if (Object.keys(settings).some(key => !['interpreter', 'import_roots', 'packages', 'python_version'].includes(key))) throw Error('unsupported Python environment field');
    if (settings.python_version !== undefined && (typeof settings.python_version !== 'string'
        || !/^3\.(8|9|10|11|12|13|14)$/.test(settings.python_version))) throw Error('python_version must be a qualified syntax target from 3.8 through 3.14');
    const interpreter = real(settings.interpreter);
    const roots: string[] = (settings.import_roots ?? []).map(real);
    const packages = (settings.packages ?? []).map((p: any) => ({...p, root: real(p.root)}));
    for (const p of packages) {
        if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(p.name) || typeof p.module !== 'string') throw Error('invalid package mapping');
        p.name = p.name.toLowerCase().replace(/[-_.]+/g, '-');
    }
    const inputs = new Map<string, string>();
    const sourceFiles = new Set<string>();
    // One immutable text/UTF-8 offset index per queried source file. The input
    // fence below still re-reads and validates every declared input afterwards.
    const sourceIndex = new Map<string, {text: string, offsets: Uint32Array}>();
    function decodeSource(file: string) {
        const bytes = fs.readFileSync(file);
        try {
            // Retain a BOM so byte offsets still address the original source.
            return new TextDecoder('utf-8', {fatal: true, ignoreBOM: true}).decode(bytes);
        } catch {
            throw Error('source is not valid UTF-8: ' + file);
        }
    }
    function indexedSource(file: string) {
        const cached = sourceIndex.get(file);
        if (cached) return cached;
        const text = decodeSource(file);
        const offsets = new Uint32Array(text.length + 1);
        let bytes = 0;
        for (let i = 0; i < text.length;) {
            const point = text.codePointAt(i)!;
            if (point > 0xffff) {
                offsets[i + 1] = bytes + 3; // A sliced lone surrogate encodes as U+FFFD.
                bytes += 4; i += 2;
            } else {
                bytes += point < 0x80 ? 1 : point < 0x800 ? 2 : 3; i++;
            }
            offsets[i] = bytes;
        }
        const value = {text, offsets};
        sourceIndex.set(file, value);
        return value;
    }
    function remember(file: string) {
        if (inputs.has(file)) return;
        const bytes = fs.readFileSync(file);
        inputs.set(file, digest(bytes));
    }
    const pathIgnored = makeIgnoreChecker(root);
    const typeshed = real(ENTMOOT_PYRIGHT_TYPESHED);
    // Fence explicit source and import universes before any analysis, including stubs.
    // Skip inventory-equivalent dirs and paths matching the publication root's gitignore.
    function freeze(
        dir: string,
        verify = false,
        observed = new Set<string>(),
        seen = new Set<string>(),
    ) {
        dir = real(dir);
        if (seen.has(dir)) return observed;
        seen.add(dir);
        for (const entry of fs.readdirSync(dir, {withFileTypes: true}).sort((a, b) => a.name < b.name ? -1 : a.name > b.name ? 1 : 0)) {
            const file = path.join(dir, entry.name);
            if (pathIgnored(file)) continue;
            if (entry.isSymbolicLink()) {
                // Project-local links still require materialization. Ambient import
                // roots (Windows site-packages junctions) resolve to their target.
                if (inside(root, file)) {
                    throw Error('linked Python inputs require a qualified materialization: ' + dir);
                }
                let target: string;
                try { target = real(file); } catch { continue; }
                try {
                    const st = fs.statSync(target);
                    if (st.isDirectory()) freeze(target, verify, observed, seen);
                    else if (st.isFile() && (/\.(py|pyi)$/.test(target) || ['py.typed', 'VERSIONS'].includes(entry.name))) {
                        observed.add(target);
                        if (!verify) { remember(target); sourceFiles.add(target); }
                    }
                } catch { /* dangling ambient link */ }
                continue;
            }
            if (entry.isDirectory()) freeze(file, verify, observed, seen);
            else if (entry.isFile() && (/\.(py|pyi)$/.test(file) || ['py.typed', 'VERSIONS'].includes(entry.name))) {
                const key = real(file);
                observed.add(key);
                if (!verify) { remember(key); sourceFiles.add(key); }
            }
        }
        return observed;
    }
    const universes = new Set([root, ...roots, ...packages.map((p: any) => p.root), typeshed]);
    const tracked = [...freeze(root)].filter(p => /\.pyi?$/.test(p));
    for (const dir of universes) if (dir !== root) freeze(dir);
    remember(interpreter);
    remember(process.execPath);
    if (environment) remember(environment);
    remember(__filename);
    // Import roots can include a project-local venv. Their files are fenced
    // evidence, not additional publication source. Decode only analyzed source
    // and files used for coordinate attachment, not every installed test fixture.
    for (const file of tracked) decodeSource(file);
    const temp = new RealTempFile();
    const console = new NullConsole();
    const sp = createServiceProvider(new PyrightFileSystem(createFromRealFileSystem(temp, console)), console, temp);
    const uri = (p: string) => Uri.file(p, sp);
    // A publication may cover a package subtree; compiler module identity still
    // uses its explicitly declared import root, not the inventory scan root.
    const analysisRoot = roots.find(importRoot => inside(importRoot, root)) ?? root;
    const config = new ConfigOptions(uri(analysisRoot));
    // Resolve only explicitly fenced import roots. Ambient installations are not
    // an implicit authority for this publication.
    class ExplicitHost extends FullAccessHost {
        getPythonSearchPaths(pythonPath: any, log: any) {
            const result = super.getPythonSearchPaths(pythonPath, log);
            return {...result, paths: roots.map(uri)};
        }
    }
    const host = new ExplicitHost(sp);
    const pythonVersion = host.getPythonVersion(uri(interpreter), []);
    if (!pythonVersion) throw Error('cannot identify configured Python interpreter');
    config.initializeFromJson({pythonPath: interpreter, extraPaths: roots, typeshedPath: typeshed,
        pythonVersion: settings.python_version ?? PythonVersion.toString(pythonVersion),
        useLibraryCodeForTypes: true, typeCheckingMode: 'off'}, uri(analysisRoot), sp, host);
    config.pythonPath = uri(interpreter);
    config.typeshedPath = uri(typeshed);
    const program = new Program(new ImportResolver(sp, config, host), config, sp, undefined, true);
    const calls: any[] = [];
    const diagnostics: string[] = [];
    const surfaces: any[] = [];
    const references: any[] = [];
    let referencesComplete = true;
    const boundaryPolicy = 'pyright.function_header_utf8.v1';
    const targetEvidence = new WeakMap<object, any>();
    const declarationTargets = new WeakMap<object, any>();
    const definitionPaths = new Map<string, string>();
    const declarationAttempts: any[] = [];
    const exportAttempts: any[] = [];
    const providerErrors: any[] = [];
    // Preserve provider metadata before filtering unsupported declarations.
    // AST handles and cyclic ownership stay process-local; primitive flags,
    // native ranges, module names and original URI spelling remain lossless.
    function nativeDeclaration(decl: any) {
        if (!decl) return null;
        const properties = Object.fromEntries(Object.entries(decl).filter(([key, value]) =>
            key === 'range' || value === null || ['string', 'number', 'boolean'].includes(typeof value)));
        return {...properties, uri: decl.uri?.toString(), node_span_utf16: decl.node ?
            {start: decl.node.start, length: decl.node.length, node_type: decl.node.nodeType} : null};
    }
    try {
        program.setTrackedFiles(tracked.map(uri));
        // Bind each tracked module once. Evaluation remains lazy and cached by Program.
        for (const file of tracked) {
            const source = program.getBoundSourceFile(uri(file));
            for (const diagnostic of source?.getParseDiagnostics() ?? []) diagnostics.push('parse_error:' + file + ':' + diagnostic.message);
            for (const imported of source?.getImports() ?? []) {
                if (!imported.isImportFound) diagnostics.push('missing_import:' + file + ':' + imported.importName);
            }
        }
        function importRoute(decl: any, value: any): any {
            if (decl.type !== DeclarationType.Alias || !decl.symbolName || !decl.uri) return value;
            const file = real(decl.uri.getFilePath());
            const mappings = packages.filter((p: any) => inside(p.root, file));
            if (mappings.length !== 1) return value;
            const pkg = mappings[0];
            const relative = portable(path.relative(pkg.root, file)).replace(/\.pyi?$/, '').replace(/\/?__init__$/, '').replaceAll('/', '.');
            const module = [pkg.module, relative].filter(Boolean).join('.');
            if (program.getBoundSourceFile(uri(file))?.getModuleName() !== module) return value;
            const evidence = targetEvidence.get(value);
            // A forwarding distribution cannot borrow the original package's
            // coordinate. Keep its native declaration but decline the stitch.
            if (!evidence || evidence.pkg?.name !== pkg.name) {
                const unsupported = {...value};
                targetEvidence.set(unsupported, {...evidence, supported: false});
                return unsupported;
            }
            const routed = {...value, symbol: [module, decl.symbolName].filter(Boolean).join('.')};
            targetEvidence.set(routed, {...evidence, declarationKey: evidence.declarationKey + ':' + routed.symbol});
            return routed;
        }
        function target(decl: any, capture?: any[], navigate = false): any {
            const attempt = {original: nativeDeclaration(decl), resolved: null as any, navigation: [] as any[]};
            capture?.push(attempt);
            const resolved = program.evaluator?.resolveAliasDeclaration(decl, true);
            attempt.resolved = nativeDeclaration(resolved);
            if (!resolved || ![DeclarationType.Function, DeclarationType.Class].includes(resolved.type) || !resolved.uri || !resolved.node) return undefined;
            if (navigate && resolved.uri.getFilePath().endsWith('.pyi')) {
                const native = program.getSourceMapper(resolved.uri, CancellationToken.None, false, false).findDeclarations(resolved);
                attempt.navigation = native.map(nativeDeclaration);
                const unique = [...new Map(native.map(item => [JSON.stringify(nativeDeclaration(item)), item])).values()];
                const stub = target(resolved);
                if (unique.length === 1 && unique[0].type === resolved.type && unique[0].uri?.getFilePath().endsWith('.py')) {
                    const implementation = target(unique[0]);
                    const declared = stub && targetEvidence.get(stub);
                    const implemented = implementation && targetEvidence.get(implementation);
                    // Native navigation is not source/stub contract equality.
                    // Retain the stub authority for boundary comparisons.
                    if (declared?.pkg && implemented?.pkg?.name === declared.pkg.name
                        && resolved.moduleName === unique[0].moduleName) {
                        const mapped = {...implementation, symbol: stub.symbol};
                        targetEvidence.set(mapped, {...declared,
                            declarationKey: implemented.declarationKey + ':stub:' + declared.declarationKey});
                        return importRoute(decl, mapped);
                    }
                }
                return stub && importRoute(decl, stub);
            }
            const cached = declarationTargets.get(resolved);
            if (cached) return importRoute(decl, cached);
            const nativePath = resolved.uri.getFilePath();
            const file = definitionPaths.get(nativePath) ?? real(nativePath);
            definitionPaths.set(nativePath, file);
            if (!inputs.has(file)) throw Error('definition outside fenced input universe: ' + file);
            const stub = file.endsWith('.pyi');
            const {text, offsets} = indexedSource(file);
            const node = resolved.node;
            const names = [node.d.name.d.value];
            const classes: any[] = [];
            let nestedFunction = false;
            for (let parent = node.parent; parent; parent = parent.parent) {
                if ([ParseNodeType.Function, ParseNodeType.Class].includes(parent.nodeType)) names.unshift(parent.d.name.d.value);
                if (parent.nodeType === ParseNodeType.Class) classes.unshift(parent);
                if (parent.nodeType === ParseNodeType.Function) nestedFunction = true;
            }
            const matches = packages.filter((p: any) => inside(p.root, file));
            const mapped = matches.length === 1 ? matches[0] : undefined;
            const relative = mapped ? portable(path.relative(mapped.root, file)).replace(/\.pyi?$/, '').replace(/\/?__init__$/, '').replaceAll('/', '.') : '';
            const mappedModule = mapped ? [mapped.module, relative].filter(Boolean).join('.') : undefined;
            const module = resolved.moduleName;
            // A configured source-root coordinate never replaces the compiler's
            // module identity. Conflicting layouts remain unqualified.
            const pkg = mappedModule === module ? mapped : undefined;
            const result = {path: inside(root, file) ? portable(path.relative(root, file)) : undefined,
                origin_file: file,
                start_byte: offsets[node.start],
                end_byte: offsets[node.start + node.length],
                symbol: [module, ...names].filter(Boolean).join('.'), kind: stub ? 'stub' : resolved.type === DeclarationType.Class ? 'type' : 'callable',
                package_role: pkg ? 'module' : 'unknown', ecosystem: 'python',
                module_path: pkg?.name};
            const nativeDecorators = (node.d.decorators ?? []).map((decorator: any) => {
                const expression = decorator.d.expr;
                const name = expression?.nodeType === ParseNodeType.Name ? expression : undefined;
                const declarations = name ? program.evaluator?.getDeclInfoForNameNode(name)?.decls ?? [] : [];
                const resolved = declarations.map((declaration: any) => program.evaluator?.resolveAliasDeclaration(declaration, true));
                const builtin = resolved.length > 0 && resolved.every((declaration: any) =>
                    declaration?.moduleName === 'builtins' && ['staticmethod', 'classmethod', 'property'].includes(declaration.node?.d?.name?.d?.value));
                return {text: text.slice(decorator.start, decorator.start + decorator.length),
                    builtin, descriptor: builtin && resolved.some((declaration: any) => declaration.node?.d?.name?.d?.value === 'property'),
                    declarations: resolved.map(nativeDeclaration)};
            });
            // Alias resolution is compiler evidence: use the canonical native
            // declaration key, never infer identity from the public alias text.
            let supported = !nestedFunction && nativeDecorators.every((decorator: any) => decorator.builtin)
                && (classes.length ? publicMember(node.d.name.d.value) : !node.d.name.d.value.startsWith('_'))
                && !classes.some(parent => parent.d.decorators?.length || parent.d.name.d.value.startsWith('_'));
            let policy = classes.length ? 'pyright.method_header_utf8.v1' : boundaryPolicy;
            let header = [...classes, node].map(item => text.slice(item.start, item.d.suite.start)).join('\n');
            let constructionOpen = false;
            if (nativeDecorators.length) {
                policy = 'pyright.builtin_decorated_method_headers.v1';
                header = nativeDecorators.map((decorator: any) => decorator.text).join('\n') + '\n' + header;
            }
            if (resolved.type === DeclarationType.Class) {
                policy = 'pyright.class_constructor_headers.v1';
                // Inherited/metaclass construction is not a constructor dispatch proof.
                supported &&= !(node.d.arguments?.length) && classes.length === 0;
                const constructors = node.d.suite.d.statements.filter((item: any) => item.nodeType === ParseNodeType.Function
                    && ['__init__', '__new__'].includes(item.d.name.d.value));
                supported &&= constructors.every((item: any) => !item.d.decorators?.length)
                    && new Set(constructors.map((item: any) => item.d.name.d.value)).size === constructors.length
                    && node.d.suite.d.statements.every((item: any) => item.nodeType === ParseNodeType.Function
                        || (item.nodeType === ParseNodeType.StatementList && item.d.statements.every((part: any) =>
                            part.nodeType === ParseNodeType.Pass || part.nodeType === ParseNodeType.StringList)));
                header += '\n' + constructors.map((item: any) => text.slice(item.start, item.d.suite.start)).join('\n');
                if (!supported && !node.d.decorators?.length && classes.length === 0) {
                    const nativeClass = program.evaluator?.getTypeOfClass(node)?.classType;
                    if (nativeClass) {
                        // A native MRO is declaration evidence, never a proof of
                        // inherited/metaclass __call__ dispatch.
                        constructionOpen = true;
                        supported = !node.d.name.d.value.startsWith('_');
                        policy = 'pyright.class_mro_declarations.v1';
                        const mro = nativeClass.shared.mro.map((base: any) => ({
                            name: base.shared?.fullName, type: program.evaluator?.printType(base)}));
                        const metaclass = nativeClass.shared.effectiveMetaclass;
                        header += '\nmro:' + JSON.stringify(mro) + '\nmetaclass:'
                            + (metaclass ? program.evaluator?.printType(metaclass) : 'unknown');
                        for (const statement of node.d.suite.d.statements)
                            header += '\n' + text.slice(statement.start, statement.nodeType === ParseNodeType.Function
                                ? statement.d.suite.start : statement.start + statement.length);
                        exportAttempts.push({class_declaration: nativeDeclaration(resolved), mro,
                            effective_metaclass: metaclass ? program.evaluator?.printType(metaclass) : null,
                            construction: 'open'});
                    }
                }
            }
            if (stub) policy = policy.replace('pyright.', 'pyright.stub_');
            targetEvidence.set(result, {pkg, supported, policy, decorators: nativeDecorators, constructionOpen,
                declarationKey: JSON.stringify([file, node.start, node.length, resolved.type]), boundary: supported ?
                digest(Buffer.from(policy + '\n' + header)) : undefined});
            declarationTargets.set(resolved, result);
            return importRoute(decl, result);
        }
        for (const pkg of packages) {
            if (!inside(root, pkg.root)) {
                // Consumer publications may map a dependency outside the analyzed
                // root; only same-root packages contribute export surfaces here.
                // Keep this out of `diagnostics` so references_complete stays honest.
                providerErrors.push({
                    kind: 'package_root_outside_analysis_root',
                    package: pkg.name,
                    root: portable(pkg.root),
                    analysis_root: portable(root),
                });
                continue;
            }
            const surface = {package: {ecosystem: 'python', name: pkg.name},
                export_universes: [boundaryPolicy, 'pyright.method_header_utf8.v1', 'pyright.class_constructor_headers.v1',
                    'pyright.stub_function_header_utf8.v1', 'pyright.stub_method_header_utf8.v1', 'pyright.stub_class_constructor_headers.v1',
                    'pyright.builtin_decorated_method_headers.v1', 'pyright.stub_builtin_decorated_method_headers.v1']
                    .concat(['pyright.inherited_member_route.v1', 'pyright.stub_inherited_member_route.v1'])
                    .concat(['pyright.class_mro_declarations.v1', 'pyright.stub_class_mro_declarations.v1'])
                    .map(boundary_policy => ({symbol_scheme: 'python.compiler.symbol', boundary_policy})),
                completeness: 'complete_relative_to_universe', exports: [] as any[], limitations: [] as string[]};
            const exportedSymbols = new Set<string>();
            const inspectedClasses = new Set<string>();
            const moduleFiles = [...inputs.keys()].filter(file => inside(pkg.root, file) && /\.(py|pyi)$/.test(file));
            if (!moduleFiles.length) surface.limitations.push('no_native_package_modules');
            for (const file of moduleFiles) {
                const parsed = program.getParseResults(uri(file));
                if (!parsed) { surface.limitations.push('module_unbound'); continue; }
                const tree = parsed.parserOutput.parseTree;
                const relative = portable(path.relative(pkg.root, file)).replace(/\.pyi?$/, '').replace(/\/?__init__$/, '').replaceAll('/', '.');
                const declaredModule = [pkg.module, relative].filter(Boolean).join('.');
                if (program.getBoundSourceFile(uri(file))?.getModuleName() !== declaredModule) {
                    surface.limitations.push('package_module_coordinate_mismatch');
                }
                const info = program.analyzerNodeInfoReader.get(tree);
                const scope = info?.scope;
                if (!scope) { surface.limitations.push('module_scope_unavailable'); continue; }
                const dunderAll = info?.dunderAllInfo;
                // Static string __all__ closes the public export universe under this
                // policy. Unsupported/dynamic forms keep the surface partial.
                const closedAll = dunderAll && !dunderAll.usesUnsupportedDunderAllForm
                    ? dunderAll.names as string[]
                    : undefined;
                if (dunderAll?.usesUnsupportedDunderAllForm) {
                    surface.limitations.push('explicit_all_and_reexports_unassessed');
                }
                if (scope.symbolTable.has('__getattr__')) surface.limitations.push('dynamic_module_exports');
                const isDunderAllTarget = (expr: any): boolean =>
                    (expr?.nodeType === ParseNodeType.Name && expr.d.value === '__all__')
                    || (expr?.nodeType === ParseNodeType.TypeAnnotation && isDunderAllTarget(expr.d.valueExpr));
                const statementAllowedUnderClosedAll = (statement: any): boolean => {
                    const items = statement.nodeType === ParseNodeType.StatementList
                        ? statement.d.statements
                        : [statement];
                    return items.every((item: any) => {
                        if ([ParseNodeType.Pass, ParseNodeType.StringList].includes(item.nodeType)) return true;
                        if ([ParseNodeType.Import, ParseNodeType.ImportFrom].includes(item.nodeType)) {
                            return true;
                        }
                        if (item.nodeType === ParseNodeType.Assignment) return isDunderAllTarget(item.d.leftExpr);
                        if (item.nodeType === ParseNodeType.AugmentedAssignment) {
                            return isDunderAllTarget(item.d.leftExpr);
                        }
                        if (item.nodeType === ParseNodeType.Call) {
                            const left = item.d.leftExpr;
                            return left?.nodeType === ParseNodeType.MemberAccess
                                && left.d.leftExpr?.nodeType === ParseNodeType.Name
                                && left.d.leftExpr.d.value === '__all__';
                        }
                        return false;
                    });
                };
                for (const statement of tree.d.statements) {
                    if ([ParseNodeType.Function, ParseNodeType.Class].includes(statement.nodeType)) continue;
                    if (statement.nodeType === ParseNodeType.StatementList && statement.d.statements.every((item: any) =>
                        item.nodeType === ParseNodeType.StringList || item.nodeType === ParseNodeType.Pass)) continue;
                    if (closedAll && statementAllowedUnderClosedAll(statement)) continue;
                    surface.limitations.push('module_export_mutation_or_statement_unassessed');
                }
                const emitExport = (decl: any, name: string, inherited?: string) => {
                    const value = target(decl);
                    const evidence = value && targetEvidence.get(value);
                    if (!evidence?.supported || !evidence.pkg || evidence.pkg.name !== pkg.name || !value.path) {
                        surface.limitations.push('export_boundary_or_coordinate_unassessed:' + name); return;
                    }
                    const route = [declaredModule, name].filter(Boolean).join('.');
                    if (evidence.constructionOpen) surface.limitations.push('class_mro_declaration_does_not_close_construction:' + name);
                    const policy = inherited === undefined ? evidence.policy
                        : file.endsWith('.pyi') ? 'pyright.stub_inherited_member_route.v1' : 'pyright.inherited_member_route.v1';
                    const boundary = inherited === undefined ? evidence.boundary
                        : digest(Buffer.from(policy + '\n' + evidence.boundary + '\n' + inherited));
                    const routeKey = route + ':' + policy;
                    if (!exportedSymbols.has(routeKey)) {
                        exportedSymbols.add(routeKey);
                        surface.exports.push({path: value.path, start_byte: value.start_byte, end_byte: value.end_byte,
                            symbol: route, boundary_policy: policy, boundary_digest: boundary});
                    }
                };
                const emitNamedExport = (name: string) => {
                    const symbol = scope.symbolTable.get(name);
                    if (!symbol) {
                        surface.limitations.push('export_name_missing_from_module:' + name);
                        return;
                    }
                    const decls = symbol.getDeclarations();
                    exportAttempts.push({package: pkg.name, module_uri: uri(file).toString(), export_name: name,
                        externally_hidden: symbol.isExternallyHidden(), declarations: decls.map(nativeDeclaration)});
                    if (decls.length !== 1) {
                        surface.limitations.push('ambiguous_export_declarations:' + name); return;
                    }
                    emitExport(decls[0], name);
                    const resolved = program.evaluator?.resolveAliasDeclaration(decls[0], true);
                    if (resolved?.type === DeclarationType.Class && resolved.node) {
                        const classRoute = [declaredModule, name, file.endsWith('.pyi') ? 'stub' : 'source'].join('.');
                        if (inspectedClasses.has(classRoute)) return;
                        inspectedClasses.add(classRoute);
                        const members = new Map(program.analyzerNodeInfoReader.get(resolved.node)?.scope?.symbolTable ?? []);
                        const inherited = new Map<string, string>();
                        const classType = program.evaluator?.getTypeOfClass(resolved.node)?.classType;
                        if (classType) for (const [memberName, member] of getClassFieldsRecursive(classType)) {
                            // Implicit object/builtin methods are not source exports.
                            // Native MRO decides which inherited declaration wins.
                            const native = member.symbol.getDeclarations();
                            if (!members.has(memberName) && native.length && native.every(declaration =>
                                declaration.uri && packages.some((mapping: any) => inside(mapping.root, declaration.uri.getFilePath())))) {
                                members.set(memberName, member.symbol);
                                const source = indexedSource(real(resolved.uri.getFilePath())).text;
                                inherited.set(memberName, source.slice(resolved.node.start, resolved.node.d.suite.start)
                                    + '\n' + program.evaluator?.printType(program.evaluator.getTypeOfMember(member)));
                            }
                        }
                        if (!members) { surface.limitations.push('class_member_scope_unavailable:' + name); return; }
                        for (const [memberName, member] of members) {
                            if (!publicMember(memberName)) continue;
                            const declarations = member.getDeclarations();
                            exportAttempts.push({package: pkg.name, module_uri: resolved.uri.toString(), export_name: name + '.' + memberName,
                                externally_hidden: member.isExternallyHidden(), declarations: declarations.map(nativeDeclaration)});
                            if (declarations.length === 1) emitExport(declarations[0], name + '.' + memberName, inherited.get(memberName));
                            else surface.limitations.push('ambiguous_member_export:' + name + '.' + memberName);
                        }
                    }
                };
                if (closedAll) {
                    for (const name of closedAll) emitNamedExport(name);
                } else {
                    for (const [name] of scope.symbolTable) {
                        if (name.startsWith('_')) continue;
                        emitNamedExport(name);
                    }
                }
            }
            surface.limitations = [...new Set(surface.limitations)].sort();
            // Missing imports inside function bodies do not change this policy's
            // native module exports or syntactic headers. Module-level imports
            // already keep the surface partial above. Retain all diagnostics and
            // incomplete reference coverage independently of export closure.
            if (diagnostics.some(message => !message.startsWith('missing_import:'))) surface.limitations.push('provider_diagnostics_present');
            if (surface.limitations.length) surface.completeness = 'partial';
            surfaces.push(surface);
        }
        for (const file of tracked) {
            if (file.endsWith('.pyi')) continue;
            const parsed = program.getParseResults(uri(file));
            if (!parsed) { diagnostics.push('parse_unavailable: ' + file); continue; }
            const {offsets} = indexedSource(file);
            class Calls extends ParseTreeWalker {
                captureScalar(node: any) {
                    if (request.canon && canonExpressions.length < 100000) {
                        const result = program.evaluator?.getTypeOfExpression(node);
                        const name = result?.type?.shared?.fullName;
                        const sort = name === 'builtins.bool' ? {kind:'bool'}
                            : name === 'builtins.int' ? {kind:'integer', bits:null, signed:true} : null;
                        if (sort) canonExpressions.push({path:portable(path.relative(root, file)), start_byte:offsets[node.start], end_byte:offsets[node.start + node.length], sort, overflow:sort.kind === 'integer' ? 'unbounded' : null});
                    }
                    return true;
                }
                visitName(node: any) { return this.captureScalar(node); }
                visitNumber(node: any) { return this.captureScalar(node); }
                visitBinaryOperation(node: any) { return this.captureScalar(node); }
                visitCall(node: any) {
                    const expr = node.d.leftExpr;
                    const memberCall = expr.nodeType === ParseNodeType.MemberAccess;
                    const name = memberCall ? expr.d.member : expr;
                    let targets: any[] = [];
                    let gap: string | undefined;
                    const declarations: any[] = [];
                    try {
                        if (name.nodeType === ParseNodeType.Name) {
                            const decls = program.evaluator?.getDeclInfoForNameNode(name)?.decls ?? [];
                            const mapped = decls.map(decl => target(decl, declarations, true));
                            targets = mapped.filter(Boolean);
                            if (mapped.length !== targets.length) {
                                targets = [];
                                gap = decls.some(decl => decl.type === DeclarationType.Param)
                                    ? 'indirect_call' : 'unsupported_declaration_or_stub';
                            }
                            if (targets.some(value => value.kind === 'stub')) gap = 'unsupported_construct';
                            // Union receivers can repeat the very same inherited
                            // declaration. Preserve every native attempt above,
                            // but count each exact source declaration once.
                            targets = [...new Map(targets.map(value =>
                                [targetEvidence.get(value).declarationKey, value])).values()];
                        } else {
                            // Calling a computed value keeps the call site; its
                            // runtime target is outside declaration lookup.
                            gap = 'indirect_call';
                        }
                    } catch (e) {
                        gap = 'provider_error'; diagnostics.push(String(e));
                        providerErrors.push({path: portable(path.relative(root, file)), start_utf16: node.start,
                            message: String(e), stack: e instanceof Error ? e.stack : null});
                    }
                    const call: any = {path: portable(path.relative(root, file)),
                        start_byte: offsets[node.start],
                        end_byte: offsets[node.start + node.length], targets,
                        gap: gap ?? (targets.length === 0 ? 'unresolved_or_dynamic' : undefined)};
                    let ownerCursor = node.parent;
                    while (ownerCursor) {
                        if (ownerCursor.nodeType === ParseNodeType.Function) {
                            call.owner = {
                                path: call.path,
                                start_byte: offsets[ownerCursor.start],
                                end_byte: offsets[ownerCursor.start + ownerCursor.length],
                            };
                            break;
                        }
                        ownerCursor = ownerCursor.parent;
                    }
                    if (targets.some(value => (targetEvidence.get(value)?.decorators ?? []).some((decorator: any) =>
                        !decorator.builtin || decorator.descriptor)))
                        call.gap = 'dynamic_dispatch';
                    if (targets.some(value => targetEvidence.get(value)?.constructionOpen)) call.gap = 'dynamic_dispatch';
                    if (!call.gap && call.owner && targets.length === 1
                        && (targets[0].kind || 'callable') === 'callable') {
                        call.dispatch = {
                            kind: 'closed',
                            policy_id: memberCall
                                ? 'entmoot.python.direct-member.v1'
                                : 'entmoot.python.direct-function.v1',
                            evidence_pointer: '/calls',
                            assumptions: [
                                'selected pyright import roots and interpreter',
                                memberCall
                                    ? 'source-level unique member invocation; decorator, runtime mutation, and foreign replacement are outside this qualification'
                                    : 'source-level unique function invocation; decorator, runtime mutation, and foreign replacement are outside this qualification',
                            ],
                        };
                    }
                    calls.push(call);
                    declarationAttempts.push({path: call.path, start_byte: call.start_byte, end_byte: call.end_byte,
                        declarations, gap: call.gap ?? null});
                    if (targets.length !== 1 || call.gap) referencesComplete = false;
                    for (const value of targets.filter(value => !value.path)) {
                        const evidence = targetEvidence.get(value);
                        if (!evidence?.pkg || !evidence.supported || call.gap) {
                            referencesComplete = false; continue;
                        }
                        references.push({path: call.path, start_byte: call.start_byte, end_byte: call.end_byte,
                            target: {package: {ecosystem: 'python', name: evidence.pkg.name},
                                symbol: {scheme: 'python.compiler.symbol', spelling: value.symbol}},
                            boundary_policy: evidence.policy, expected_boundary: evidence.boundary});
                    }
                    return true;
                }
            }
            new Calls().walk(parsed.parserOutput.parseTree);
        }
        for (const info of program.getSourceFileInfoList()) {
            const candidate = info.sourceFile.getUri().getFilePath();
            const file = fs.existsSync(candidate) ? real(candidate) : candidate;
            if (fs.existsSync(file) && !inputs.has(file)) diagnostics.push('unfenced_import: ' + file);
        }
        for (const [file, expected] of inputs) {
            if (digest(fs.readFileSync(file)) !== expected) throw Error('input changed during batch: ' + file);
        }
        const after = new Set<string>();
        for (const dir of universes) freeze(dir, true, after);
        if (after.size !== sourceFiles.size || [...after].some(file => !sourceFiles.has(file))) throw Error('Python input file set changed during batch');
        const unfenced = diagnostics.some(message => message.startsWith('unfenced_import:'));
        if (unfenced || diagnostics.some(message => message.startsWith('parse_error:'))) {
            for (const surface of surfaces) {
                surface.completeness = 'partial'; surface.limitations.push('input_or_parse_gap');
            }
        }
        return {compiler_version: 'pyright-program/1.1.414',
            tool_environment: [{role: 'python', executable: interpreter,
                selection: discovered?.selection ?? {kind: 'environment', variable: 'ENTMOOT_PYTHON_ENVIRONMENT'},
                version: PythonVersion.toString(pythonVersion)}],
            calls, inputs: [...inputs.keys()].map(portable), diagnostics,
            invalid_environment: unfenced || diagnostics.some(message => message.startsWith('parse_error:')), build_context: {...(request.canon ? {canon_expressions: canonExpressions} : {}), partial: true,
                package_api: {surfaces, references, references_complete: referencesComplete && diagnostics.length === 0,
                    limitations: ['static_function_method_and_declared_constructor_headers_only',
                        'runtime_mutation_inherited_construction_decorators_and_import_only_sites_unassessed']},
                policy: 'pyright.program.declarations.v1', interpreter, settings,
                python_version: PythonVersion.toString(pythonVersion), node_version: process.version, platform: process.platform, architecture: process.arch,
                syntax_python_version: settings.python_version ?? PythonVersion.toString(pythonVersion),
                pyright_source_revision: 'a07bbdeae085f59396acee8739c7d862dd3a1ff1',
                environment_digest: digest(configBytes), input_digests: Object.fromEntries(inputs),
                declaration_attempts: declarationAttempts,
                export_attempts: exportAttempts,
                provider_errors: providerErrors,
                limitations: ['stub_source_mapping_is_native_navigation_not_runtime_equivalence', 'export_enumeration_may_be_partial',
                    'package_coordinates_operator_declared_and_module_checked', 'project_typechecker_config_not_loaded',
                    'typecheck_success_not_compatibility', 'selected_import_roots_recorded_not_package_or_runtime_qualification']}};
    } finally { program.dispose(); temp.dispose(); }
}

try {
    process.stdout.write(JSON.stringify(run(JSON.parse(fs.readFileSync(0, 'utf8')))));
} catch (error) {
    process.stdout.write(JSON.stringify({invalid_environment: true, diagnostics: ['invalid_environment: ' + String(error)],
        build_context: {provider_error: {message: String(error), stack: error instanceof Error ? error.stack : null}}}));
}
