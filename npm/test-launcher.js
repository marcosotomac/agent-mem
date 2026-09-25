const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { ensureBinary, getTargetAsset } = require('./bin/agent-mem');

async function main() {
  const asset = getTargetAsset();
  if (!asset.endsWith('.tar.gz')) return;

  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'agent-mem-npm-test-'));
  try {
    const staging = path.join(root, 'staging');
    const release = path.join(root, 'release');
    fs.mkdirSync(staging);
    fs.mkdirSync(release);
    fs.copyFileSync(path.join(__dirname, '..', 'target', 'debug', 'agent-mem'), path.join(staging, 'agent-mem'));
    execFileSync('tar', ['-czf', path.join(release, asset), '-C', staging, 'agent-mem']);
    const archive = fs.readFileSync(path.join(release, asset));
    const digest = crypto.createHash('sha256').update(archive).digest('hex');
    fs.writeFileSync(path.join(release, 'SHA256SUMS.txt'), `${digest}  ${asset}\n`);

    const fixtureDownload = async (url, destination) => {
      fs.copyFileSync(path.join(release, path.basename(url)), destination);
    };
    const options = {
      cacheDir: path.join(root, 'cache'),
      releaseBaseUrl: 'https://fixture.invalid/release',
      download: fixtureDownload,
    };
    const bin = await ensureBinary(options);
    assert.equal(execFileSync(bin, ['--version'], { encoding: 'utf8' }).trim(),
      `agent-mem ${require('./package.json').version}`);
    assert.equal(await ensureBinary(options), bin);

    fs.appendFileSync(bin, 'corrupt');
    await ensureBinary(options);
    assert.equal(crypto.createHash('sha256').update(fs.readFileSync(bin)).digest('hex'),
      fs.readFileSync(`${bin}.sha256`, 'utf8').trim());

    fs.writeFileSync(path.join(release, 'SHA256SUMS.txt'), `${'0'.repeat(64)}  ${asset}\n`);
    await assert.rejects(
      ensureBinary({ ...options, cacheDir: path.join(root, 'bad-cache') }),
      /SHA-256 mismatch/
    );
    process.stdout.write('npm launcher archive install and checksum rejection passed\n');
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

main().catch((error) => {
  process.stderr.write(`${error.stack}\n`);
  process.exitCode = 1;
});
