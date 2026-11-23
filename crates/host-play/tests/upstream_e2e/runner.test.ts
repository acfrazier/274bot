import { expect, test } from 'bun:test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

test('a case reads upstream Git HEAD while writing to its evidence directory', () => {
    const scratch = mkdtempSync(join(tmpdir(), 'upstream-e2e-git-'));
    try {
        const upstream = join(scratch, 'upstream');
        const output = join(scratch, 'output');
        const id = 'nature-runner-coins-739-live';
        mkdirSync(join(upstream, 'e2e'), { recursive: true });
        const git = (...args: string[]) => execFileSync('git', args, { cwd: upstream, encoding: 'utf8' });
        git('init', '--quiet', '--template=', '--separate-git-dir', join(scratch, 'metadata'));
        git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
            '-c', `core.hooksPath=${join(scratch, 'no-hooks')}`,
            'commit', '--quiet', '--allow-empty', '--no-gpg-sign', '-m', 'fixture');
        const head = git('rev-parse', 'HEAD').trim();
        writeFileSync(join(upstream, 'e2e', `${id}.ts`), `
            import { execFileSync } from 'node:child_process';
            const sourceCommit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
            await Bun.write('case-evidence.json', JSON.stringify({ sourceCommit }));
        `);

        // Invoke the same child entry as the corpus after it validates the source hash.
        const child = Bun.spawnSync([process.execPath, join(import.meta.dir, 'runner.ts'), '--case', id], {
            env: { ...process.env, UEP_CASES: id, UEP_UPSTREAM: upstream, UEP_OUTPUT: output },
            stdout: 'pipe',
            stderr: 'pipe',
        });
        expect(child.exitCode, child.stderr.toString()).toBe(0);
        expect(JSON.parse(readFileSync(join(output, id, 'case-evidence.json'), 'utf8'))).toEqual({ sourceCommit: head });
    } finally {
        rmSync(scratch, { recursive: true, force: true });
    }
});
