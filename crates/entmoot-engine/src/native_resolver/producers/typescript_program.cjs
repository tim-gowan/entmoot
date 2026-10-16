// Entmoot-owned TypeScript Program adapter. Native objects stay process-local.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const {blake3} = require('@noble/hashes/blake3');
const {bytesToHex} = require('@noble/hashes/utils');
const compilerPath = process.env.ENTMOOT_TYPESCRIPT_LIBRARY || require.resolve('typescript');
const ts = require(compilerPath);
const digest = bytes => 'blake3:' + bytesToHex(blake3(bytes));
const portable = value => value.replaceAll('\\', '/');
const inside = (root, file) => {
    const rel = path.relative(root, file);
    return !path.isAbsolute(rel) && rel !== '..' && !rel.startsWith('..' + path.sep);
};
const scheme = 'javascript.compiler.symbol';

function run(request) {
    if (!['4.9.5', '5.0.4', '5.5.4', '5.9.3', '6.0.3'].includes(ts.version)) throw Error('unqualified TypeScript Program version (7.0 exposes no compiler API): ' + ts.version);
    const root = fs.realpathSync(request.root);
    const reads = new Map(), encodings = new Map(), probes = new Map(), diagnostics = [], attempts = [];
    // Observe every compiler-host lookup, including misses and directory results.
    // Recheck the same queries after analysis; this is an observed-query fence,
    // not a transactional filesystem snapshot or proof of runtime resolution.
    const operations = ['fileExists', 'directoryExists', 'readDirectory', 'getDirectories', 'realpath'];
    const hostIO = {useCaseSensitiveFileNames: ts.sys.useCaseSensitiveFileNames,
        getCurrentDirectory: () => root};
    for (const op of operations) if (ts.sys[op]) hostIO[op] = (...args) => {
        args[0] = path.resolve(root, args[0]);
        const key = JSON.stringify([op, args]);
        const value = ts.sys[op](...args);
        const result = JSON.stringify(value);
        if (probes.has(key) && probes.get(key).result !== result) throw Error('lookup changed during batch: ' + args[0]);
        probes.set(key, {operation: op, arguments: args, result});
        return value;
    };
    hostIO.readFile = file => {
        file = path.resolve(root, file);
        let bytes;
        try { bytes = fs.readFileSync(file); }
        catch (error) { if (!['ENOENT', 'ENOTDIR', 'EISDIR'].includes(error.code)) throw error; }
        const value = bytes === undefined ? null : digest(bytes);
        if (reads.has(file) && reads.get(file) !== value) throw Error('input changed during batch: ' + file);
        reads.set(file, value);
        if (!bytes) return undefined;
        encodings.set(file, {utf16: bytes[0] === 0xff && bytes[1] === 0xfe || bytes[0] === 0xfe && bytes[1] === 0xff,
            bom: bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf ? 3 : 0});
        if (bytes[0] === 0xff && bytes[1] === 0xfe) return bytes.subarray(2).toString('utf16le');
        if (bytes[0] === 0xfe && bytes[1] === 0xff) return Buffer.from(bytes.subarray(2)).swap16().toString('utf16le');
        return bytes.toString('utf8').replace(/^\uFEFF/, '');
    };
    for (const file of [...Object.keys(require.cache), __filename, process.execPath]) reads.set(file, digest(fs.readFileSync(file)));
    const configFile = process.env.ENTMOOT_TYPESCRIPT_CONFIG || path.join(root, 'tsconfig.json');
    const configText = hostIO.readFile(configFile);
    if (configText === undefined) throw Error('explicit tsconfig.json required: ' + configFile);
    const config = ts.parseConfigFileTextToJson(configFile, configText);
    if (config.error) throw Error(ts.flattenDiagnosticMessageText(config.error.messageText, '\n'));
    const parsed = ts.parseJsonConfigFileContent(config.config, hostIO, path.dirname(configFile),
        {noEmit: true, incremental: false}, configFile);
    if (parsed.errors.length) throw Error(parsed.errors.map(d => ts.flattenDiagnosticMessageText(d.messageText, '\n')).join('\n'));
    const host = Object.assign(ts.createCompilerHost(parsed.options, true), hostIO);
    host.useCaseSensitiveFileNames = () => ts.sys.useCaseSensitiveFileNames;
    host.writeFile = () => { throw Error('producer cannot emit compiler output'); };
    const program = ts.createProgram({rootNames: parsed.fileNames, options: parsed.options,
        projectReferences: parsed.projectReferences, host});
    const checker = program.getTypeChecker();
    const nativeDiagnostics = ts.getPreEmitDiagnostics(program);
    for (const d of nativeDiagnostics) diagnostics.push(JSON.stringify({
        code: d.code, category: ts.DiagnosticCategory[d.category], path: d.file?.fileName,
        start_utf16: d.start, length_utf16: d.length, message: ts.flattenDiagnosticMessageText(d.messageText, '\n')}));
    const invalid = program.getSyntacticDiagnostics().length > 0 || program.getOptionsDiagnostics().length > 0;
    const offsets = new Map();
    function span(node) {
        const source = node.getSourceFile(), file = path.resolve(source.fileName);
        if (encodings.get(file)?.utf16) throw Error('UTF-16 source has no qualified structural inventory: ' + file);
        let index = offsets.get(file);
        if (!index) {
            index = new Uint32Array(source.text.length + 1);
            let bytes = encodings.get(file)?.bom || 0;
            index[0] = bytes;
            for (let i = 0; i < source.text.length;) {
                const point = source.text.codePointAt(i);
                if (point > 0xffff) { index[i + 1] = bytes + 3; bytes += 4; i += 2; }
                else { bytes += point < 128 ? 1 : point < 2048 ? 2 : 3; i++; }
                index[i] = bytes;
            }
            offsets.set(file, index);
        }
        return {file, path: inside(root, file) && !program.isSourceFileFromExternalLibrary(source)
                ? portable(path.relative(root, file)) : undefined,
            start_byte: index[node.getStart(source)], end_byte: index[node.end]};
    }
    const packageCache = new Map();
    function packageOf(file) {
        if (packageCache.has(file)) return packageCache.get(file);
        let dir = path.dirname(file), pkg;
        while (true) {
            const candidate = path.join(dir, 'package.json');
            if (hostIO.fileExists(candidate)) {
                const data = JSON.parse(hostIO.readFile(candidate));
                if (typeof data.name === 'string' && /^(@[a-z0-9._-]+\/)?[a-z0-9._-]+$/.test(data.name))
                    pkg = {root: dir, name: data.name, version: data.version, manifest: data};
                break;
            }
            const parent = path.dirname(dir);
            if (parent === dir) break;
            dir = parent;
        }
        packageCache.set(file, pkg);
        return pkg;
    }
    function resolveAlias(symbol) {
        const seen = new Set();
        while (symbol && symbol.flags & ts.SymbolFlags.Alias) {
            if (seen.has(symbol)) return undefined;
            seen.add(symbol); symbol = checker.getAliasedSymbol(symbol);
        }
        return symbol;
    }
    function declarations(symbol) {
        const resolved = resolveAlias(symbol);
        const defs = resolved?.getDeclarations() || [];
        if (defs.length === 1 && ts.isShorthandPropertyAssignment(defs[0]))
            return declarations(checker.getShorthandAssignmentValueSymbol(defs[0]));
        if (defs.length === 1 && (ts.isBinaryExpression(defs[0]) || ts.isPropertyAssignment(defs[0]) || ts.isPropertyAccessExpression(defs[0])
            || ts.isVariableDeclaration(defs[0]))) {
            const signatures = checker.getTypeOfSymbolAtLocation(resolved, defs[0]).getCallSignatures();
            if (signatures.length === 1 && signatures[0].declaration) return [signatures[0].declaration];
        }
        return defs;
    }
    function header(node) {
        const source = node.getSourceFile(), start = node.getStart(source), cuts = [];
        function visit(child) {
            if (ts.isFunctionLike(child) && child.body) {
                cuts.push([child.body.getStart(source) - start, child.body.end - start]); return;
            }
            ts.forEachChild(child, visit);
        }
        visit(node);
        let text = source.text.slice(start, node.end);
        for (const [begin, end] of cuts.sort((a, b) => b[0] - a[0])) text = text.slice(0, begin) + '{}' + text.slice(end);
        return text;
    }
    function declaration(node) {
        const location = span(node), stdlib = program.isSourceFileDefaultLibrary(node.getSourceFile());
        const pkg = stdlib ? undefined : packageOf(location.file);
        const names = [];
        for (let part = node; part && !ts.isSourceFile(part); part = part.parent) {
            if (part.name && (part === node || ts.isClassLike(part) || ts.isInterfaceDeclaration(part)))
                names.unshift(part.name.getText());
        }
        const callable = ts.isFunctionLike(node) && !!node.body;
        const authority = node.getSourceFile().isDeclarationFile ? 'declaration' : 'source';
        const boundary_policy = 'typescript.' + authority + '_declaration_headers.v1';
        // Only compiler-owned declarations can supply a contract. Assignment
        // expressions and computed properties retain evidence but no boundary.
        const supported = ts.isFunctionLike(node) || ts.isClassLike(node) || ts.isInterfaceDeclaration(node)
            || ts.isTypeAliasDeclaration(node) || ts.isEnumDeclaration(node);
        return {...location, pkg, names, node, authority, supported, stdlib,
            key: JSON.stringify([location.file, location.start_byte, location.end_byte]),
            kind: authority === 'declaration' ? 'type' : callable ? 'callable' : 'type',
            boundary_policy, boundary_text: supported ? header(node) : undefined};
    }
    const sourceFiles = program.getSourceFiles().filter(source => !program.isSourceFileDefaultLibrary(source));
    const routesByKey = new Map(), modules = new Map(), surfaces = new Map(), publicRoutes = [];
    function routesForFile(source, pkg) {
        const file = path.resolve(source.fileName);
        if (pkg.manifest.exports === undefined) return [{route: './' + portable(path.relative(pkg.root, file)), conditions: []}];
        const result = [];
        function walk(value, route, conditions) {
            if (typeof value === 'string') {
                if (!value.startsWith('./') || value.includes('*')) return;
                const native = ts.resolveModuleName(value, path.join(pkg.root, 'package.json'), parsed.options, host).resolvedModule;
                if (native && path.resolve(native.resolvedFileName) === file) result.push({route, conditions});
            } else if (Array.isArray(value)) value.forEach((part, i) => walk(part, route, [...conditions, 'alternative:' + i]));
            else if (value && typeof value === 'object') for (const [key, part] of Object.entries(value))
                walk(part, key.startsWith('.') ? key : route, key.startsWith('.') ? conditions : [...conditions, key]);
        }
        walk(pkg.manifest.exports, '.', []);
        return result;
    }
    function publicMember(symbol) {
        return declarations(symbol).every(node => !node.name || !ts.isPrivateIdentifier(node.name))
            && declarations(symbol).every(node => !(ts.getCombinedModifierFlags(node) & (ts.ModifierFlags.Private | ts.ModifierFlags.Protected)));
    }
    for (const source of sourceFiles) {
        const file = path.resolve(source.fileName), pkg = packageOf(file);
        const moduleSymbol = checker.getSymbolAtLocation(source);
        if (!pkg || !moduleSymbol) continue;
        const fileRoutes = routesForFile(source, pkg);
        if (!fileRoutes.length) continue;
        const exports = checker.getExportsOfModule(moduleSymbol);
        const routeMap = new Map();
        modules.set(moduleSymbol, routeMap);
        let surface = surfaces.get(pkg.root);
        if (!surface) {
            surface = {package: {ecosystem: 'javascript', name: pkg.name},
                export_universes: ['source', 'declaration'].map(authority => ({symbol_scheme: scheme,
                    boundary_policy: 'typescript.' + authority + '_declaration_headers.v1'})),
                completeness: 'partial', exports: [], limitations: ['selected_program_export_universe', 'runtime_export_mutation_unassessed']};
            surfaces.set(pkg.root, surface);
        }
        function emit(symbol, exportName, publicOwner) {
            const defs = declarations(symbol);
            attempts.push({module: file, exported_name: exportName, flags: symbol.flags,
                declarations: defs.map(node => ({...span(node), syntax_kind: ts.SyntaxKind[node.kind]}))});
            if (defs.length !== 1) { surface.limitations.push('overloaded_or_merged_export:' + exportName); return; }
            const value = declaration(defs[0]);
            if (publicOwner) value.boundary_text = header(publicOwner) + '\nexposed_type:'
                + checker.typeToString(checker.getTypeOfSymbolAtLocation(symbol, publicOwner), publicOwner,
                    ts.TypeFormatFlags.NoTruncation) + '\n' + value.boundary_text;
            if (!value.supported || !value.pkg || value.pkg.name !== pkg.name) {
                surface.limitations.push('unsupported_or_cross_package_export:' + exportName); return;
            }
            const entries = [];
            // The selected native entry file disambiguates import/require/types
            // branches without pretending those authorities are interchangeable.
            for (const route of fileRoutes) {
                const spelling = pkg.name + ':' + route.route + '@' + portable(path.relative(pkg.root, file)) + '#' + exportName;
                const entry = {...value, spelling, moduleSymbol, exportName};
                if (!entries.some(e => e.spelling === spelling)) entries.push(entry);
                publicRoutes.push({package: pkg.name, route: route.route, conditions: route.conditions,
                    entry_file: file, export_name: exportName, declaration: value.key, authority: value.authority});
            }
            routeMap.set(exportName, entries);
            for (const entry of entries) {
                if (!routesByKey.has(value.key)) routesByKey.set(value.key, []);
                routesByKey.get(value.key).push(entry);
                if (value.path && inside(root, pkg.root)) surface.exports.push({
                    path: value.path, start_byte: value.start_byte, end_byte: value.end_byte, symbol: entry.spelling,
                    boundary_policy: value.boundary_policy, boundary_text: value.boundary_text});
            }
            if (ts.isClassLike(defs[0]) || ts.isInterfaceDeclaration(defs[0])) {
                for (const member of checker.getPropertiesOfType(checker.getDeclaredTypeOfSymbol(resolveAlias(symbol))))
                    if (publicMember(member)) emitMember(member, exportName, defs[0]);
            }
        }
        function emitMember(symbol, owner, declaration) {
            // emit may recurse only for nested named type members, bounded by
            // actual class/interface declarations, never arbitrary object types.
            const defs = declarations(symbol);
            if (defs.length === 1 && ts.isFunctionLike(defs[0])) emit(symbol, owner + '.' + symbol.getName(), declaration);
        }
        for (const symbol of exports) emit(symbol, symbol.getName());
    }
    function importEntries(expr) {
        const symbol = checker.getSymbolAtLocation(expr);
        for (const decl of symbol?.getDeclarations() || []) {
            let importNode = decl;
            while (importNode && !ts.isImportDeclaration(importNode)) importNode = importNode.parent;
            if (!importNode) continue;
            const module = checker.getSymbolAtLocation(importNode.moduleSpecifier);
            const name = ts.isImportSpecifier(decl) ? (decl.propertyName || decl.name).text
                : ts.isImportClause(decl) ? 'default' : undefined;
            if (name) return modules.get(module)?.get(name);
        }
        if (ts.isPropertyAccessExpression(expr)) {
            const receiver = checker.getSymbolAtLocation(expr.expression);
            const decl = receiver?.getDeclarations()?.find(ts.isNamespaceImport);
            if (decl) {
                const module = checker.getSymbolAtLocation(decl.parent.parent.moduleSpecifier);
                return modules.get(module)?.get(expr.name.text);
            }
        }
        return undefined;
    }
    const calls = [], references = [], canonExpressions = [];
    let referencesComplete = nativeDiagnostics.length === 0;
    for (const source of sourceFiles.filter(source => inside(root, path.resolve(source.fileName))
        && !program.isSourceFileFromExternalLibrary(source) && !source.isDeclarationFile)) {
        function visit(node) {
            if (request.canon && nativeDiagnostics.length === 0 && canonExpressions.length < 100000
                && ((ts.isIdentifier(node) || ts.isBinaryExpression(node) || ts.isBigIntLiteral(node) || node.kind === ts.SyntaxKind.TrueKeyword || node.kind === ts.SyntaxKind.FalseKeyword) || ts.isParameter(node))) {
                const type = checker.getTypeAtLocation(node);
                const sort = type.flags & ts.TypeFlags.BooleanLike ? {kind:'bool'}
                    : type.flags & ts.TypeFlags.BigIntLike ? {kind:'integer', bits:null, signed:true} : null;
                if (sort) canonExpressions.push({...span(node), sort, overflow: sort.kind === 'integer' ? 'unbounded' : null});
            }
            if (ts.isCallExpression(node) || ts.isNewExpression(node)) {
                const location = span(node), signature = checker.getResolvedSignature(node);
                const native = signature?.declaration;
                const value = native ? declaration(native) : undefined;
                const targets = value ? [{
                    origin_file: value.file,
                    path: value.path, start_byte: value.start_byte, end_byte: value.end_byte,
                    symbol: (value.pkg?.name || '') + ':' + portable(path.relative(value.pkg?.root || root, value.file))
                        + '#' + value.names.join('.'),
                    kind: value.kind, package_role: value.stdlib ? 'stdlib' : value.pkg ? 'module' : 'unknown',
                    ecosystem: 'javascript', module_path: value.pkg?.name, package_version: value.pkg?.version}] : [];
                const call = {path: location.path, start_byte: location.start_byte, end_byte: location.end_byte, targets,
                    gap: native && (ts.isFunctionTypeNode(native) || ts.isCallSignatureDeclaration(native)) ? 'indirect_call'
                        : !value ? 'unresolved_or_dynamic' : value.authority === 'declaration' ? 'unsupported_construct' : undefined};
                let ownerCursor = node.parent;
                while (ownerCursor) {
                    if (ts.isFunctionDeclaration(ownerCursor) || ts.isMethodDeclaration(ownerCursor)
                        || ts.isFunctionExpression(ownerCursor) || ts.isArrowFunction(ownerCursor)
                        || ts.isConstructorDeclaration(ownerCursor) || ts.isGetAccessor(ownerCursor)
                        || ts.isSetAccessor(ownerCursor)) {
                        const ownerSpan = span(ownerCursor);
                        if (ownerSpan.path) {
                            call.owner = {
                                path: ownerSpan.path,
                                start_byte: ownerSpan.start_byte,
                                end_byte: ownerSpan.end_byte,
                            };
                        }
                        break;
                    }
                    ownerCursor = ownerCursor.parent;
                }
                if (!call.gap && call.owner && targets.length === 1
                    && (targets[0].kind || 'callable') === 'callable'
                    && value && value.authority !== 'declaration'
                    && !(native && (ts.isFunctionTypeNode(native) || ts.isCallSignatureDeclaration(native)))) {
                    call.dispatch = {
                        kind: 'closed',
                        policy_id: 'entmoot.typescript.direct-function.v1',
                        evidence_pointer: '/calls',
                        assumptions: [
                            'selected TypeScript program and tsconfig',
                            'source-level unique function signature; call signatures, runtime mutation, and foreign replacement are outside this qualification',
                        ],
                    };
                }
                calls.push(call);
                let entries = importEntries(node.expression) || (value && routesByKey.get(value.key)) || [];
                entries = [...new Map(entries.map(entry => [entry.spelling, entry])).values()];
                if (!value || !value.supported || entries.length !== 1 || call.gap) referencesComplete = false;
                if (value && !value.path && entries.length === 1 && value.supported && !call.gap) {
                    const entry = entries[0];
                    targets[0].symbol = entry.spelling;
                    references.push({path: call.path, start_byte: call.start_byte, end_byte: call.end_byte,
                        target: {package: {ecosystem: 'javascript', name: entry.pkg.name},
                            symbol: {scheme, spelling: entry.spelling}},
                        boundary_policy: entry.boundary_policy, boundary_text: entry.boundary_text});
                }
                attempts.push({path: call.path, start_byte: call.start_byte, end_byte: call.end_byte,
                    signature: signature ? checker.signatureToString(signature) : null,
                    declaration: value?.key, routes: entries.map(entry => entry.spelling),
                    dispatch: 'open', authority: value?.authority});
            }
            ts.forEachChild(node, visit);
        }
        visit(source);
    }
    for (const [file, expected] of reads) {
        let bytes;
        try { bytes = fs.readFileSync(file); }
        catch (error) { if (!['ENOENT', 'ENOTDIR', 'EISDIR'].includes(error.code)) throw error; }
        const actual = bytes === undefined ? null : digest(bytes);
        if (expected !== actual) throw Error('input changed during batch: ' + file);
    }
    for (const probe of probes.values())
        if (JSON.stringify(ts.sys[probe.operation](...probe.arguments)) !== probe.result)
            throw Error('lookup changed during batch: ' + probe.arguments[0]);
    const ownSurfaces = [...surfaces].filter(([directory]) => inside(root, directory)
        && !path.relative(root, directory).split(path.sep).includes('node_modules')).map(([, surface]) => {
        surface.exports = [...new Map(surface.exports.map(row => [row.symbol, row])).values()].sort((a, b) => a.symbol.localeCompare(b.symbol));
        surface.limitations = [...new Set(surface.limitations)].sort();
        return surface;
    });
    return {compiler_version: 'typescript/' + ts.version, calls, diagnostics, invalid_environment: invalid,
        inputs: [...reads.keys()].filter(file => reads.get(file) !== null).sort(),
        build_context: {...(request.canon ? {canon_expressions: canonExpressions} : {}), partial: true, policy: 'typescript.program.declarations.v1',
            compiler_options: parsed.options, config_file: configFile, project_references: parsed.projectReferences || [],
            node_version: process.version, platform: process.platform, architecture: process.arch,
            input_digests: Object.fromEntries([...reads].sort(([a], [b]) => a.localeCompare(b))),
            lookup_observations: [...probes.values()].map(probe => ({...probe, result: JSON.parse(probe.result)})),
            input_closure: 'observed_compiler_host_queries_revalidated',
            native_attempts: attempts, public_routes: publicRoutes,
            package_api: {surfaces: ownSurfaces, references, references_complete: referencesComplete,
                limitations: ['selected_program_only', 'runtime_dispatch_and_decorator_effects_open',
                    'source_and_declaration_authorities_distinct', 'typecheck_success_not_compatibility']}}};
}
try {
    process.stdout.write(JSON.stringify(run(JSON.parse(fs.readFileSync(0, 'utf8')))));
} catch (error) {
    process.stdout.write(JSON.stringify({invalid_environment: true, diagnostics: ['invalid_environment: ' + String(error)],
        build_context: {provider_error: {message: String(error), stack: error.stack}}}));
}
