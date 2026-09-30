#!/usr/bin/env node
'use strict';
// Dev-time only. Runtime code does not depend on this tool or the frozen tree.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { execFileSync } = require('node:child_process');
const ts = require('typescript');
const { rollup } = require('rollup');
const { dts } = require('rollup-plugin-dts');
const here = __dirname;
const crate = path.resolve(here, '..');
const frozen = process.env.RS2B0T;
const frozenRevision = '00d39a17e056df6c5e461f3f2cfd3598ff9720b6';
const frozenRepository = 'https://github.com/rs2b2t/rs2b0t.git';
const frozenRevisionLabel = frozenRevision.slice(0, 10);
if (!frozen) throw new Error(`Set RS2B0T to the frozen rs2b0t ${frozenRevisionLabel} source. Run npm ci in compat-js first.`);
function verifyFrozenRevision(root) {
    let sourceRoot;
    try {
        sourceRoot = fs.realpathSync(root);
    } catch (error) {
        throw new Error(`RS2B0T revision check failed: expected ${frozenRevisionLabel} (${frozenRevision}) from ${frozenRepository}; cannot resolve source path: ${error.message}`);
    }
    try {
        const provenance = JSON.parse(fs.readFileSync(path.join(path.dirname(sourceRoot), 'REFERENCE.json'), 'utf8'));
        if (
            provenance.repository === frozenRepository &&
            provenance.commit === frozenRevision &&
            provenance.method === `git archive ${frozenRevision} | tar -x` &&
            typeof provenance.export === 'string' &&
            path.isAbsolute(provenance.export) &&
            fs.realpathSync(provenance.export) === sourceRoot
        ) return;
    } catch {}
    try {
        const gitRoot = fs.realpathSync(execFileSync('git', ['-C', sourceRoot, 'rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim());
        if (gitRoot !== sourceRoot) throw new Error(`git toplevel ${gitRoot} does not match RS2B0T path ${sourceRoot}`);
        const actualRevision = execFileSync('git', ['-C', gitRoot, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
        if (actualRevision !== frozenRevision) throw new Error(`git HEAD is ${actualRevision || '(empty)'}`);
    } catch (error) {
        throw new Error(`RS2B0T revision check failed: expected ${frozenRevisionLabel} (${frozenRevision}) from ${frozenRepository}; ${error.message}`);
    }
}
verifyFrozenRevision(frozen);
if (ts.version !== '5.8.3') throw new Error('The generator requires TypeScript 5.8.3; run npm ci.');
const printer = ts.createPrinter({ newLine: ts.NewLineKind.LineFeed });
const parse = (text, file = 'module.d.ts') => ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true);
const print = (node, sf) => printer.printNode(ts.EmitHint.Unspecified, node, sf);
const modifiers = n => (n.modifiers || []).map(m => m.kind);
const name = n => n.name && (n.name.text || n.name.getText());
const registryText = fs.readFileSync(path.join(crate, 'src/shim/modules.rs'), 'utf8');
const registry = [...registryText.matchAll(/Module::new\(\s*"\/rs2b0t\/bot\/([^"]+)",\s*include_str!\("([^"]+)"\)/g)]
    .map(m => ({ module: m[1], shim: m[2] })).filter(m => !m.module.startsWith('scripts/'));
if (!registry.length) throw new Error('No shim registry modules found');
const prelude = parse(registryText.match(/const PRELUDE: &str = r#"([\s\S]*?)"#;/)[1], 'prelude.js');
const preludeClasses = new Map();
for (const s of prelude.statements) {
    if (ts.isExpressionStatement(s) && ts.isBinaryExpression(s.expression) && ts.isClassExpression(s.expression.right)) {
        preludeClasses.set(name(s.expression.right), s.expression.right);
    }
}
function surface(text) {
    const sf = parse(text, 'shim.js');
    const exports = new Map();
    const locals = new Map();
    const imports = new Map();
    const reexports = new Map();
    for (const s of sf.statements) {
        if (ts.isImportDeclaration(s) && s.importClause) {
            const c = s.importClause;
            if (c.name) imports.set(c.name.text, { from: s.moduleSpecifier.text, imported: 'default' });
            if (c.namedBindings && ts.isNamedImports(c.namedBindings)) for (const e of c.namedBindings.elements) imports.set(e.name.text, { from: s.moduleSpecifier.text, imported: (e.propertyName || e.name).text });
        }
        if (ts.isVariableStatement(s)) for (const d of s.declarationList.declarations) locals.set(name(d), d.initializer);
        if (ts.isClassDeclaration(s) || ts.isFunctionDeclaration(s)) locals.set(name(s), s);
    }
    for (const s of sf.statements) {
        if (modifiers(s).includes(ts.SyntaxKind.ExportKeyword)) {
            if (ts.isVariableStatement(s)) for (const d of s.declarationList.declarations) exports.set(name(d), d.initializer);
            else if (name(s)) exports.set(name(s), s);
        }
        if (modifiers(s).includes(ts.SyntaxKind.DefaultKeyword)) exports.set('default', s);
        if (ts.isExportAssignment(s) && ts.isIdentifier(s.expression)) exports.set('default', locals.get(s.expression.text) || null);
        if (ts.isExportDeclaration(s) && s.exportClause && ts.isNamedExports(s.exportClause)) {
            for (const e of s.exportClause.elements) {
                const imported = (e.propertyName || e.name).text;
                exports.set(e.name.text, locals.get(imported) || preludeClasses.get(imported) || null);
                const source = s.moduleSpecifier ? { from: s.moduleSpecifier.text, imported } : imports.get(imported);
                if (source) reexports.set(e.name.text, source);
            }
        }
    }
    return { sf, exports, locals, imports, reexports };
}
const surfaces = new Map(registry.map(r => [r.module, surface(fs.readFileSync(path.join(crate, 'src/shim', r.shim), 'utf8'))]));
for (let pass = 0; pass < surfaces.size; pass++) {
    let changed = false;
    for (const [module, live] of surfaces) for (const [symbol, source] of live.reexports) {
        if (live.exports.get(symbol)) continue;
        const target = path.posix.normalize(path.posix.join(path.posix.dirname(module), source.from));
        const node = surfaces.get(target)?.exports.get(source.imported);
        if (node) { live.exports.set(symbol, node); changed = true; }
    }
    if (!changed) break;
}
function members(node, live) {
    if (!node) return null;
    if (ts.isNewExpression(node) && node.expression.getText() === 'Proxy') {
        const target = members(node.arguments?.[0], live);
        const handler = node.arguments?.[1];
        // ownKeys can expose host-posted keys absent from an empty proxy target.
        if (target?.size === 0 && ts.isObjectLiteralExpression(handler) && handler.properties.some(p => name(p) === 'ownKeys')) return null;
        return target;
    }
    if (ts.isNewExpression(node) && ts.isIdentifier(node.expression) && live?.locals.has(node.expression.text)) return members(live.locals.get(node.expression.text), live);
    if (ts.isCallExpression(node) && node.expression.getText() === 'proxy') return members(node.arguments?.[1], live);
    if (ts.isObjectLiteralExpression(node)) return new Set(node.properties.map(name));
    if (ts.isClassDeclaration(node) || ts.isClassExpression(node)) {
        const set = new Set(node.members.map(m => ts.isConstructorDeclaration(m) ? 'constructor' : name(m)));
        function fields(n) {
            if (ts.isBinaryExpression(n) && ts.isPropertyAccessExpression(n.left) && n.left.expression.kind === ts.SyntaxKind.ThisKeyword) set.add(n.left.name.text);
            ts.forEachChild(n, fields);
        }
        ts.forEachChild(node, fields);
        const base = node.heritageClauses?.[0]?.types[0]?.expression;
        if (base && live) for (const field of members(live.locals.get(base.getText()) || preludeClasses.get(base.getText()), live) || []) if (field !== 'constructor') set.add(field);
        return set;
    }
    return null;
}
const overlayFiles = ['overlay.json', 'overlay-native.json', 'overlay-scene.json', 'overlay-not-impl.json'];
const overlay = overlayFiles.flatMap(f => JSON.parse(fs.readFileSync(path.join(here, f), 'utf8')));
const typeNamesByModule = new Map();
const used = new Set();
function objectTypes(sf, live) {
    const locals = new Map(sf.statements.filter(s => name(s)).map(s => [name(s), s]));
    function shape(type, allowed, seen = new Set()) {
        if (!type) return type;
        if (ts.isTypeLiteralNode(type)) return type;
        if (!ts.isTypeReferenceNode(type)) return type;
        const key = type.typeName.getText(sf);
        if (key === 'Readonly' && type.typeArguments?.length === 1) return shape(type.typeArguments[0], allowed, seen);
        if (key === 'Record' && allowed && type.typeArguments?.length === 2) return ts.factory.createTypeLiteralNode([...allowed].map(key => ts.factory.createPropertySignature(undefined, ts.factory.createStringLiteral(key), undefined, type.typeArguments[1])));
        if (seen.has(key)) return type;
        seen.add(key);
        const target = locals.get(key);
        if (!target) return allowed ? ts.factory.createTypeLiteralNode([...allowed].map(field => ts.factory.createPropertySignature(undefined, ts.factory.createStringLiteral(field), undefined, ts.factory.createIndexedAccessTypeNode(type, ts.factory.createLiteralTypeNode(ts.factory.createStringLiteral(field)))))) : type;
        if (ts.isTypeAliasDeclaration(target)) return shape(target.type, allowed, seen);
        if (!ts.isInterfaceDeclaration(target) && !ts.isClassDeclaration(target)) return type;
        return ts.factory.createTypeLiteralNode(target.members.filter(m => !ts.isConstructorDeclaration(m) && !modifiers(m).some(k => k === ts.SyntaxKind.PrivateKeyword || k === ts.SyntaxKind.ProtectedKeyword)).map(m => {
            if (ts.isMethodDeclaration(m)) return ts.factory.createMethodSignature(undefined, m.name, m.questionToken, m.typeParameters, m.parameters, m.type);
            if (ts.isPropertyDeclaration(m)) return ts.factory.createPropertySignature(m.modifiers?.filter(x => x.kind === ts.SyntaxKind.ReadonlyKeyword), m.name, m.questionToken, m.type);
            if (ts.isGetAccessorDeclaration(m)) return ts.factory.createPropertySignature([ts.factory.createModifier(ts.SyntaxKind.ReadonlyKeyword)], m.name, undefined, m.type);
            return m;
        }));
    }
    return sf.statements.map(s => {
        if (!ts.isVariableStatement(s)) return s;
        const declarations = s.declarationList.declarations.map(d => ts.factory.updateVariableDeclaration(d, d.name, d.exclamationToken, shape(d.type, members(live.exports.get(name(d)), live)), d.initializer));
        return ts.factory.updateVariableStatement(s, s.modifiers, ts.factory.updateVariableDeclarationList(s.declarationList, declarations));
    }).map(s => print(s, sf)).join('\n');
}
function notImplemented(node, entry) {
    const result = entry.async ? ts.factory.createTypeReferenceNode('Promise', [ts.factory.createKeywordTypeNode(ts.SyntaxKind.NeverKeyword)]) : ts.factory.createKeywordTypeNode(ts.SyntaxKind.NeverKeyword);
    const params = node.parameters?.map(p => ts.factory.updateParameterDeclaration(p, p.modifiers, p.dotDotDotToken, p.name, p.dotDotDotToken ? p.questionToken : ts.factory.createToken(ts.SyntaxKind.QuestionToken), p.type, undefined));
    if (ts.isFunctionDeclaration(node)) return ts.factory.updateFunctionDeclaration(node, node.modifiers, node.asteriskToken, node.name, node.typeParameters, params, result, undefined);
    if (ts.isMethodSignature(node)) return ts.factory.updateMethodSignature(node, node.modifiers, node.name, node.questionToken, node.typeParameters, params, result);
    if (ts.isMethodDeclaration(node)) return ts.factory.updateMethodDeclaration(node, node.modifiers, node.asteriskToken, node.name, node.questionToken, node.typeParameters, params, result, undefined);
    throw new Error(`Not a callable not-impl selector: ${entry.id}`);
}
function transformDeclaration(node, entry) {
    if (entry.operation === 'not-impl') return notImplemented(node, entry);
    if (entry.operation === 'extends') {
        if (!ts.isClassDeclaration(node)) throw new Error(`Not a class selector: ${entry.id}`);
        const base = ts.factory.createHeritageClause(ts.SyntaxKind.ExtendsKeyword, [ts.factory.createExpressionWithTypeArguments(ts.factory.createIdentifier(entry.base), undefined)]);
        return ts.factory.updateClassDeclaration(node, node.modifiers, node.name, node.typeParameters, [base, ...(node.heritageClauses || []).filter(h => h.token !== ts.SyntaxKind.ExtendsKeyword)], node.members);
    }
    if (entry.operation !== 'optional' || !node.parameters) throw new Error(`Unsupported overlay operation: ${entry.id}`);
    const matched = new Set();
    const params = node.parameters.map(p => {
        if (!entry.parameters.includes(name(p))) return p;
        matched.add(name(p));
        return ts.factory.updateParameterDeclaration(p, p.modifiers, p.dotDotDotToken, p.name, ts.factory.createToken(ts.SyntaxKind.QuestionToken), p.type, undefined);
    });
    if (matched.size !== entry.parameters.length) throw new Error(`Unknown optional parameter: ${entry.id}`);
    if (ts.isFunctionDeclaration(node)) return ts.factory.updateFunctionDeclaration(node, node.modifiers, node.asteriskToken, node.name, node.typeParameters, params, node.type, undefined);
    if (ts.isMethodSignature(node)) return ts.factory.updateMethodSignature(node, node.modifiers, node.name, node.questionToken, node.typeParameters, params, node.type);
    if (ts.isMethodDeclaration(node)) return ts.factory.updateMethodDeclaration(node, node.modifiers, node.asteriskToken, node.name, node.questionToken, node.typeParameters, params, node.type, undefined);
    throw new Error(`Not a callable optional selector: ${entry.id}`);
}
function overlayDeclarations(text) {
    const sf = parse(text);
    function synthesize(n) {
        ts.setTextRange(n, { pos: -1, end: -1 });
        ts.forEachChild(n, synthesize);
    }
    for (const s of sf.statements) synthesize(s);
    return [...sf.statements];
}
function apply(module, text) {
    const entries = overlay.filter(e => e.module === module);
    let sf = parse(text);
    sf = parse(objectTypes(sf, surfaces.get(module)));
    let statements = [...sf.statements];
    for (const e of entries) {
        if (!e.reason || !/^crates\/script\/[^:]+:\d+/.test(e.runtime)) throw new Error(`Missing reason/runtime citation: ${e.id}`);
        if (used.has(e.id)) throw new Error(`Duplicate overlay id: ${e.id}`);
        if (e.selector === '$module') {
            statements = overlayDeclarations(e.declaration);
        } else if (e.selector === '$add') {
            statements.push(...overlayDeclarations(e.declaration));
        } else {
            const [symbol, member] = e.selector.split('.');
            let hits = 0;
            const replacement = e.declaration ? overlayDeclarations(e.declaration) : [];
            statements = statements.flatMap(s => {
                const variable = ts.isVariableStatement(s) && s.declarationList.declarations.length === 1 && name(s.declarationList.declarations[0]) === symbol;
                if (!variable && name(s) !== symbol) return [s];
                if (!member) { hits++; return e.operation ? [transformDeclaration(s, e)] : replacement; }
                const decl = variable ? s.declarationList.declarations[0] : s;
                const type = variable ? decl.type : decl;
                if (!type || (!ts.isTypeLiteralNode(type) && !ts.isClassDeclaration(type) && !ts.isInterfaceDeclaration(type))) throw new Error(`Not a member container: ${e.id}`);
                const replacementMember = overlayDeclarations(`declare class X { ${e.declaration || ''} }`)[0].members;
                const changed = type.members.flatMap(m => {
                    if ((ts.isConstructorDeclaration(m) ? 'constructor' : name(m)) !== member) return [m];
                    hits++;
                    return e.operation ? [transformDeclaration(m, e)] : [...replacementMember];
                });
                if (!hits && e.add) { changed.push(...replacementMember); hits++; }
                if (variable) {
                    const updated = ts.factory.updateVariableDeclaration(decl, decl.name, decl.exclamationToken, ts.factory.updateTypeLiteralNode(type, changed), decl.initializer);
                    return [ts.factory.updateVariableStatement(s, s.modifiers, ts.factory.updateVariableDeclarationList(s.declarationList, [updated]))];
                }
                if (ts.isClassDeclaration(s)) return [ts.factory.updateClassDeclaration(s, s.modifiers, s.name, s.typeParameters, s.heritageClauses, changed)];
                return [ts.factory.updateInterfaceDeclaration(s, s.modifiers, s.name, s.typeParameters, s.heritageClauses, changed)];
            });
            if (hits !== 1) throw new Error(`Overlay selector ${e.id} hit ${hits} nodes`);
        }
        used.add(e.id);
        // Reparse each edit so printer text ranges always belong to their source.
        sf = parse(statements.map(s => print(s, sf)).join('\n'));
        statements = [...sf.statements];
    }
    const live = surfaces.get(module);
    const localSymbols = new Set(statements.flatMap(s => ts.isVariableStatement(s) ? s.declarationList.declarations.map(name) : name(s) ? [name(s)] : []));
    return statements.flatMap(s => {
        if (ts.isExportDeclaration(s) && s.exportClause && ts.isNamedExports(s.exportClause)) {
            const kept = s.exportClause.elements.filter(e => !localSymbols.has(e.name.text) && (e.isTypeOnly || s.isTypeOnly || live.exports.has(e.name.text)));
            return kept.length ? [ts.factory.updateExportDeclaration(s, s.modifiers, s.isTypeOnly, ts.factory.updateNamedExports(s.exportClause, kept), s.moduleSpecifier, s.attributes)] : [];
        }
        if (ts.isClassDeclaration(s) || ts.isFunctionDeclaration(s) || ts.isVariableStatement(s)) {
            const symbol = ts.isVariableStatement(s) ? name(s.declarationList.declarations[0]) : name(s);
            if (modifiers(s).includes(ts.SyntaxKind.ExportKeyword) && !live.exports.has(symbol) && !modifiers(s).includes(ts.SyntaxKind.DefaultKeyword)) {
                if (ts.isClassDeclaration(s)) {
                    const fields = s.members.filter(m => !ts.isConstructorDeclaration(m) && !modifiers(m).some(k => k === ts.SyntaxKind.PrivateKeyword || k === ts.SyntaxKind.ProtectedKeyword)).map(m => {
                        if (ts.isMethodDeclaration(m)) return ts.factory.createMethodSignature(undefined, m.name, m.questionToken, m.typeParameters, m.parameters, m.type);
                        if (ts.isPropertyDeclaration(m)) return ts.factory.createPropertySignature(m.modifiers?.filter(x => x.kind === ts.SyntaxKind.ReadonlyKeyword), m.name, m.questionToken, m.type);
                        return m;
                    });
                    return [ts.factory.createInterfaceDeclaration([ts.factory.createModifier(ts.SyntaxKind.ExportKeyword)], s.name, s.typeParameters, undefined, fields)];
                }
                return [];
            }
            const allowed = members(live.exports.get(symbol) || live.locals.get(symbol), live);
            if (allowed && ts.isClassDeclaration(s)) {
                // Private/protected state is an implementation detail, not a posted model contract.
                const kept = s.members.filter(m => !modifiers(m).some(k => k === ts.SyntaxKind.PrivateKeyword || k === ts.SyntaxKind.ProtectedKeyword) && allowed.has(ts.isConstructorDeclaration(m) ? 'constructor' : name(m)));
                return [ts.factory.updateClassDeclaration(s, s.modifiers, s.name, s.typeParameters, s.heritageClauses, kept)];
            }
            if (allowed && ts.isVariableStatement(s)) {
                const d = s.declarationList.declarations[0];
                if (d.type && ts.isTypeLiteralNode(d.type)) {
                    const kept = d.type.members.filter(m => allowed.has(name(m)));
                    const updated = ts.factory.updateVariableDeclaration(d, d.name, d.exclamationToken, ts.factory.updateTypeLiteralNode(d.type, kept), d.initializer);
                    return [ts.factory.updateVariableStatement(s, s.modifiers, ts.factory.updateVariableDeclarationList(s.declarationList, [updated]))];
                }
            }
        }
        return [s];
    }).map(s => print(s, sf)).join('\n') + '\n';
}
function normalizeBundle(text) {
    const sf = parse(text);
    const exported = new Map();
    for (const s of sf.statements) if (ts.isExportDeclaration(s) && !s.moduleSpecifier && s.exportClause && ts.isNamedExports(s.exportClause)) {
        for (const e of s.exportClause.elements) exported.set((e.propertyName || e.name).text, e.name.text);
    }
    return sf.statements.flatMap(s => {
        if (ts.isExportDeclaration(s) && !s.moduleSpecifier) return [];
        if ((ts.isImportDeclaration(s) || ts.isExportDeclaration(s)) && s.moduleSpecifier?.text?.startsWith('./*')) {
            const spec = ts.factory.createStringLiteral(s.moduleSpecifier.text.slice(2));
            if (ts.isImportDeclaration(s)) return [ts.factory.updateImportDeclaration(s, s.modifiers, s.importClause, spec, s.attributes)];
            const types = typeNamesByModule.get(spec.text.slice(1)) || new Set();
            const clause = s.exportClause && ts.isNamedExports(s.exportClause) ? ts.factory.updateNamedExports(s.exportClause, s.exportClause.elements.map(e => ts.factory.updateExportSpecifier(e, e.isTypeOnly || types.has((e.propertyName || e.name).text), e.propertyName, e.name))) : s.exportClause;
            return [ts.factory.updateExportDeclaration(s, s.modifiers, s.isTypeOnly, clause, spec, s.attributes)];
        }
        const symbol = ts.isVariableStatement(s) ? name(s.declarationList.declarations[0]) : name(s);
        if (!exported.has(symbol)) return [s];
        const target = exported.get(symbol);
        if (target !== symbol && target !== 'default') throw new Error(`Unexpected local export alias: ${symbol} as ${target}`);
        const mods = [ts.factory.createModifier(ts.SyntaxKind.ExportKeyword), ...(target === 'default' && !ts.isVariableStatement(s) ? [ts.factory.createModifier(ts.SyntaxKind.DefaultKeyword)] : []), ...(s.modifiers || []).filter(m => m.kind === ts.SyntaxKind.AbstractKeyword)];
        if (ts.isClassDeclaration(s)) return [ts.factory.updateClassDeclaration(s, mods, s.name, s.typeParameters, s.heritageClauses, s.members)];
        if (ts.isFunctionDeclaration(s)) return [ts.factory.updateFunctionDeclaration(s, mods, s.asteriskToken, s.name, s.typeParameters, s.parameters, s.type, s.body)];
        if (ts.isVariableStatement(s)) return [ts.factory.updateVariableStatement(s, mods, s.declarationList), ...(target === 'default' ? [ts.factory.createExportAssignment(undefined, false, ts.factory.createIdentifier(symbol))] : [])];
        if (ts.isInterfaceDeclaration(s)) return [ts.factory.updateInterfaceDeclaration(s, mods, s.name, s.typeParameters, s.heritageClauses, s.members)];
        if (ts.isTypeAliasDeclaration(s)) return [ts.factory.updateTypeAliasDeclaration(s, mods, s.name, s.typeParameters, s.type)];
        return [s];
    }).map(s => print(s, sf)).join('\n').replace(/^declare (?=(?:abstract )?(?:class|function|const|let|var)\b)/gm, '') + '\n';
}
async function main() {
    const scratch = fs.mkdtempSync(path.join(os.tmpdir(), '274bot-dts-'));
    try {
        const root = path.join(frozen, 'src');
        const out = path.join(scratch, 'emit');
        const roots = registry.map(r => path.join(root, 'bot', r.module.replace(/\.js$/, '.ts'))).filter(f => fs.existsSync(f));
        const config = { compilerOptions: { target: 'ESNext', module: 'ESNext', moduleResolution: 'bundler', lib: ['ESNext', 'DOM', 'DOM.Iterable'], strict: true, skipLibCheck: true, baseUrl: root, paths: { '#/*': ['*'], fflate: [path.join(here, 'node_modules/fflate/lib/index.d.ts')] }, allowJs: true, typeRoots: [path.join(here, 'node_modules/@types')], declaration: true, emitDeclarationOnly: true, noEmitOnError: true, rootDir: root, outDir: out }, files: roots };
        const configPath = path.join(scratch, 'tsconfig.json');
        fs.writeFileSync(configPath, JSON.stringify(config));
        execFileSync('npx', ['-p', 'typescript@5.8.3', '--yes', 'tsc', '--pretty', 'false', '-p', configPath], { stdio: 'inherit' });
        const files = new Map(registry.map(r => [path.join(out, 'bot', r.module.replace(/\.js$/, '.d.ts')), r.module]));
        for (const [file, module] of files) {
            fs.mkdirSync(path.dirname(file), { recursive: true });
            const original = fs.existsSync(file) ? fs.readFileSync(file, 'utf8') : '';
            fs.writeFileSync(file, apply(module, original));
        }
        for (const [file, module] of files) {
            typeNamesByModule.set(module, new Set(parse(fs.readFileSync(file, 'utf8')).statements.filter(s => ts.isInterfaceDeclaration(s) || ts.isTypeAliasDeclaration(s)).map(name)));
        }
        const modules = [];
        for (const [file, module] of files) {
            const bundle = await rollup({ input: file, plugins: [dts({ compilerOptions: { baseUrl: out, paths: { '#/*': ['*'] } } })], external: id => {
                const abs = path.resolve(path.dirname(file), id).replace(/\.js$/, '.d.ts');
                return files.has(id) && id !== file || files.has(abs) && abs !== file;
            } });
            const { output } = await bundle.generate({ format: 'es', paths: id => {
                const abs = path.resolve(path.dirname(file), id).replace(/\.js$/, '.d.ts');
                const target = files.get(id) || files.get(abs);
                if (!target) throw new Error(`Unsupported external declaration dependency: ${id}`);
                return '*' + target;
            } });
            await bundle.close();
            modules.push({ module, body: normalizeBundle(output[0].code) });
        }
        const define = overlay.find(e => e.id === 'defineBot');
        const barrelPolicy = overlay.find(e => e.module === '@rs2b0t/api' && e.selector === '$barrel' && e.operation === 'runtime-values');
        if (!barrelPolicy) throw new Error('Missing named barrel runtime-value overlay');
        for (const entry of [define, barrelPolicy].filter(Boolean)) {
            if (!entry.reason || !/^crates\/script\/[^:]+:\d+/.test(entry.runtime)) throw new Error(`Missing reason/runtime citation: ${entry.id}`);
            used.add(entry.id);
        }
        const unused = overlay.filter(e => !used.has(e.id));
        if (unused.length) throw new Error(`Unused overlay entries: ${unused.map(e => e.id)}`);
        const license = fs.readFileSync(path.join(frozen, 'LICENSE'), 'utf8').trim();
        const header = `// Generated by generate.cjs from rs2b0t ${frozenRevisionLabel}; DO NOT EDIT.\n// Named host divergences are applied from overlay*.json.\n/*\nDerived script-API declarations: Copyright (c) 2026 N64Jive\n${license}\n*/\n`;
        const barrelSource = parse(fs.readFileSync(path.join(crate, 'src/shim/declared_surface.js'), 'utf8'), 'barrel.js');
        const barrel = barrelSource.statements.flatMap(s => {
            if (ts.isExportDeclaration(s)) return [print(s, barrelSource).replace(/(['"])\.\.\/\.\.\/([^'"]+)\1/g, "'*$2'")];
            if (!modifiers(s).includes(ts.SyntaxKind.ExportKeyword)) return [];
            if (ts.isVariableStatement(s)) return s.declarationList.declarations.filter(d => name(d) !== 'defineBot').map(d => {
                if (ts.isCallExpression(d.initializer) && d.initializer.expression.getText() === 'notImplValue') return `export const ${name(d)}: never;`;
                if (ts.isObjectLiteralExpression(d.initializer)) return `export const ${name(d)}: { useTeleportCatalog: boolean; policy: {useTeleports: boolean} };`;
                if (ts.isNumericLiteral(d.initializer)) return `export const ${name(d)}: ${d.initializer.text};`;
                if (ts.isCallExpression(d.initializer) && d.initializer.expression.getText() === 'proxy') {
                    const methods = members(d.initializer);
                    return `export const ${name(d)}: { ${[...methods].map(n => `${n}(...args: unknown[]): never;`).join(' ')} };`;
                }
                throw new Error(`Unmodeled barrel value: ${name(d)}`);
            });
            if (ts.isClassDeclaration(s)) return [`export class ${name(s)} { ${s.members.map(m => ts.isConstructorDeclaration(m) ? 'constructor(...args: unknown[]);' : `${modifiers(m).includes(ts.SyntaxKind.StaticKeyword) ? 'static ' : ''}${name(m)}(...args: unknown[]): never;`).join(' ')} }`];
            if (ts.isFunctionDeclaration(s)) return [`export function ${name(s)}(...args: unknown[]): never;`];
            throw new Error(`Unmodeled barrel statement: ${s.getText()}`);
        }).join('\n');
        let index = header + `declare module '@rs2b0t/api' {\n${define ? define.declaration : ''}\n${barrel}\n}\n`;
        for (const m of modules.sort((a,b) => a.module.localeCompare(b.module))) index += `\ndeclare module '*${m.module}' {\n${m.body}\n}\n`;
        const products = { 'index.d.ts': index, 'modules.json': JSON.stringify(registry, null, 2) + '\n' };
        for (const [file, content] of Object.entries(products)) {
            const dest = path.join(here, file);
            if (process.argv.includes('--check')) {
                if (!fs.existsSync(dest) || fs.readFileSync(dest, 'utf8') !== content) throw new Error(`${file} differs from fresh emit; run node generate.cjs`);
            } else fs.writeFileSync(dest, content);
        }
        console.log(`${process.argv.includes('--check') ? 'Verified' : 'Generated'} ${modules.length} served modules (${registry.filter(r => r.module.startsWith('api/')).length} api/ URLs) with ${overlay.length} named overlay entries.`);
    } finally { fs.rmSync(scratch, { recursive: true, force: true }); }
}
main().catch(e => { console.error(e); process.exitCode = 1; });
