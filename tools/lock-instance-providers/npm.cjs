// Reference selection producer, explicitly invoked outside bind/TypeScript Program.
// Registry package-lock v2/v3 only. No install, resolve, fetch, or runtime claims.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const {blake3} = require('@noble/hashes/blake3');
const {bytesToHex} = require('@noble/hashes/utils');
const digest = bytes => 'blake3:' + bytesToHex(blake3(bytes));

function select(root) {
    root = fs.realpathSync(root);
    const inputs = new Map(), absent = new Set(), paths = new Map();
    let total = 0;
    function read(file) {
        if (inputs.has(file)) return inputs.get(file);
        let size;
        try { size = fs.statSync(file).size; }
        catch (error) {
            if (error.code !== 'ENOENT') throw error;
            absent.add(file);
            return undefined;
        }
        if (size > 32 * 1024 * 1024 || total + size > 64 * 1024 * 1024) throw Error('input byte budget');
        // Bound the actual read as well as the metadata check.
        const fd = fs.openSync(file, 'r'), buffer = Buffer.alloc(size + 1);
        let offset = 0;
        try {
            while (offset < buffer.length) {
                const count = fs.readSync(fd, buffer, offset, buffer.length - offset, null);
                if (!count) break;
                offset += count;
            }
        } finally { fs.closeSync(fd); }
        if (offset !== size) throw Error('input length changed: ' + file);
        const bytes = buffer.subarray(0, size);
        total += size; inputs.set(file, bytes);
        return bytes;
    }
    const bytes = read(path.join(root, 'package-lock.json'));
    const result = {producer: 'entmoot.reference.npm', policy: 'npm.registry_slots.v1', packages: []};
    const lock = bytes === undefined ? undefined : JSON.parse(bytes.toString('utf8'));
    if (lock && ![2, 3].includes(lock.lockfileVersion)) throw Error('unsupported package-lock version');
    const lockDigest = bytes === undefined ? undefined : digest(bytes);
    const entries = Object.entries(lock?.packages || {}).sort(([a], [b]) => a.localeCompare(b));
    if (entries.length > 4096) throw Error('package budget');
    for (const [placement, entry] of entries) {
        const parts = placement.split('/');
        if (!parts.includes('node_modules') || parts.some(p => !p || p === '.' || p === '..')
            || placement.includes('\\') || path.isAbsolute(placement)) continue;
        if (entry.link || typeof entry.integrity !== 'string' || !entry.integrity
            || typeof entry.resolved !== 'string' || !/^https?:\/\//.test(entry.resolved)) continue;
        const directory = path.join(root, placement);
        let physical;
        try { physical = fs.realpathSync(directory); }
        catch (error) {
            if (error.code !== 'ENOENT') throw error;
            absent.add(directory); continue;
        }
        paths.set(directory, physical);
        // Workspace/file/git links need a different producer policy.
        if (physical !== directory) continue;
        const manifest = read(path.join(directory, 'package.json'));
        if (!manifest) continue;
        const pkg = JSON.parse(manifest.toString('utf8'));
        const installedName = placement.slice(placement.lastIndexOf('node_modules/') + 'node_modules/'.length);
        if (typeof pkg.name !== 'string' || !/^(@[a-z0-9._-]+\/)?[a-z0-9._-]+$/.test(pkg.name)
            || typeof pkg.version !== 'string' || !pkg.version
            || (entry.name || installedName) !== pkg.name || entry.version !== pkg.version) continue;
        const instance = {
            purl: 'pkg:npm/' + pkg.name.split('/').map(encodeURIComponent).join('/') + '@' + encodeURIComponent(pkg.version),
            lock_path: 'package-lock.json', lock_digest: lockDigest,
            entry: '/packages/' + placement.replaceAll('~', '~0').replaceAll('/', '~1'), placement,
        };
        const hash = blake3.create({});
        for (const part of ['entmoot.lock_instance.v1', instance.purl, instance.lock_path,
            instance.lock_digest, instance.entry, instance.placement]) {
            const value = Buffer.from(part), size = Buffer.alloc(8);
            size.writeBigUInt64LE(BigInt(value.length)); hash.update(size); hash.update(value);
        }
        instance.instance_id = bytesToHex(hash.digest());
        result.packages.push({root: placement, instance});
    }
    for (const [file, expected] of inputs) {
        const fd = fs.openSync(file, 'r'), actual = Buffer.alloc(expected.length + 1);
        let offset = 0;
        try {
            while (offset < actual.length) {
                const count = fs.readSync(fd, actual, offset, actual.length - offset, null);
                if (!count) break;
                offset += count;
            }
        } finally { fs.closeSync(fd); }
        if (!expected.equals(actual.subarray(0, offset))) throw Error('input changed: ' + file);
    }
    for (const file of absent) if (fs.existsSync(file)) throw Error('input appeared: ' + file);
    for (const [directory, physical] of paths)
        if (fs.realpathSync(directory) !== physical) throw Error('placement changed: ' + directory);
    return result;
}

if (require.main === module) {
    if (process.argv.length !== 3) throw Error('usage: node npm-lock-provider.cjs CONSUMER_ROOT');
    process.stdout.write(JSON.stringify(select(process.argv[2]), null, 2) + '\n');
}
module.exports = {select};
