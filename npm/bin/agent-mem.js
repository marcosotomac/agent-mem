#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const os = require('os');
const { spawn, spawnSync } = require('child_process');
const https = require('https');
const crypto = require('crypto');

const PKG = require('../package.json');
const VERSION = PKG.version;
const REPO = 'marcosotomac/agent-mem';

function getTargetAsset() {
  const platform = os.platform();
  const arch = os.arch();

  if (platform === 'darwin') {
    if (arch === 'arm64') return 'agent-mem-darwin-aarch64.tar.gz';
    if (arch === 'x64') return 'agent-mem-darwin-x86_64.tar.gz';
  } else if (platform === 'linux') {
    if (arch === 'arm64') return 'agent-mem-linux-aarch64.tar.gz';
    if (arch === 'x64') return 'agent-mem-linux-x86_64.tar.gz';
  } else if (platform === 'win32') {
    if (arch === 'x64') return 'agent-mem-windows-x86_64.zip';
  }

  throw new Error(`Unsupported platform: ${platform} ${arch}`);
}

function verifyBinaryVersion(binPath) {
  const result = spawnSync(binPath, ['--version'], { encoding: 'utf8' });
  if (result.error || result.status !== 0) {
    throw new Error(`Unable to execute native binary at ${binPath}`);
  }
  const expected = `agent-mem ${VERSION}`;
  if (result.stdout.trim() !== expected) {
    throw new Error(`Native binary version mismatch: expected ${expected}`);
  }
}

function download(url, dest, redirects = 0) {
  return new Promise((resolve, reject) => {
    https.get(url, (response) => {
      if (response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
        response.resume();
        if (redirects >= 5) return reject(new Error('Too many HTTPS redirects'));
        const next = new URL(response.headers.location, url);
        if (next.protocol !== 'https:') return reject(new Error('Refusing non-HTTPS redirect'));
        return download(next.href, dest, redirects + 1).then(resolve).catch(reject);
      }
      if (response.statusCode !== 200) {
        response.resume();
        return reject(new Error(`Failed to download binary: HTTP ${response.statusCode}`));
      }

      const partial = `${dest}.part-${process.pid}`;
      const file = fs.createWriteStream(partial, { mode: 0o600 });
      response.pipe(file);
      file.on('finish', () => {
        file.close(() => {
          fs.renameSync(partial, dest);
          resolve();
        });
      });
      file.on('error', (err) => {
        fs.unlink(partial, () => {});
        reject(err);
      });
    }).on('error', (err) => {
      reject(err);
    });
  });
}

function sha256(filePath) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256');
    const input = fs.createReadStream(filePath);
    input.on('error', reject);
    input.on('data', (chunk) => hash.update(chunk));
    input.on('end', () => resolve(hash.digest('hex')));
  });
}

async function verifyArchive(archivePath, asset, releaseBaseUrl, downloadFile = download) {
  const checksumsPath = `${archivePath}.sha256sums`;
  await downloadFile(`${releaseBaseUrl}/SHA256SUMS.txt`, checksumsPath);
  const line = fs.readFileSync(checksumsPath, 'utf8')
    .split(/\r?\n/)
    .find((entry) => entry.trim().endsWith(` ${asset}`));
  fs.unlinkSync(checksumsPath);
  if (!line) throw new Error(`No SHA-256 checksum published for ${asset}`);

  const expected = line.trim().split(/\s+/)[0].toLowerCase();
  const actual = await sha256(archivePath);
  if (actual !== expected) {
    fs.unlinkSync(archivePath);
    throw new Error(`SHA-256 mismatch for ${asset}`);
  }
}

async function ensureBinary(options = {}) {
  // Deliberately never fall back to PATH: `npx agent-mem@X` must execute X,
  // not an unrelated global install. This override exists for package smoke
  // tests and controlled development only, and is version checked.
  if (process.env.AGENT_MEM_BINARY) {
    const override = path.resolve(process.env.AGENT_MEM_BINARY);
    verifyBinaryVersion(override);
    return override;
  }

  const asset = getTargetAsset();
  const isWin = os.platform() === 'win32';
  const binName = isWin ? 'agent-mem.exe' : 'agent-mem';
  const cacheDir = options.cacheDir || path.join(os.homedir(), '.cache', 'agent-mem', `v${VERSION}`);
  const binPath = path.join(cacheDir, binName);
  const digestPath = `${binPath}.sha256`;

  if (fs.existsSync(binPath) && fs.existsSync(digestPath)
      && fs.readFileSync(digestPath, 'utf8').trim() === await sha256(binPath)) {
    verifyBinaryVersion(binPath);
    return binPath;
  }

  fs.mkdirSync(cacheDir, { recursive: true });
  const archivePath = path.join(cacheDir, asset);
  const releaseBaseUrl = options.releaseBaseUrl || `https://github.com/${REPO}/releases/download/v${VERSION}`;
  const downloadUrl = `${releaseBaseUrl}/${asset}`;
  const downloadFile = options.download || download;

  // Log to stderr only so stdout JSON-RPC MCP channel is NEVER polluted
  process.stderr.write(`[agent-mem] Downloading native binary v${VERSION} for ${os.platform()}-${os.arch()}...\n`);

  await downloadFile(downloadUrl, archivePath);
  await verifyArchive(archivePath, asset, releaseBaseUrl, downloadFile);

  if (asset.endsWith('.tar.gz')) {
    const result = spawnSync('tar', ['-xzf', archivePath, '-C', cacheDir], { stdio: 'ignore' });
    if (result.status !== 0) throw new Error('Failed to extract release archive');
  } else if (asset.endsWith('.zip')) {
    if (isWin) {
      const script = 'Expand-Archive -LiteralPath $args[0] -DestinationPath $args[1] -Force';
      const result = spawnSync('powershell', ['-NoProfile', '-Command', script, archivePath, cacheDir], { stdio: 'ignore' });
      if (result.status !== 0) throw new Error('Failed to extract release archive');
    } else {
      const result = spawnSync('unzip', ['-o', archivePath, '-d', cacheDir], { stdio: 'ignore' });
      if (result.status !== 0) throw new Error('Failed to extract release archive');
    }
  }

  try {
    fs.unlinkSync(archivePath);
  } catch (_) {}

  if (!isWin) {
    fs.chmodSync(binPath, 0o755);
  }

  verifyBinaryVersion(binPath);
  fs.writeFileSync(digestPath, `${await sha256(binPath)}\n`, { mode: 0o600 });

  return binPath;
}

async function main() {
  try {
    const binPath = await ensureBinary();
    const args = process.argv.slice(2);
    const child = spawn(binPath, args, { stdio: 'inherit' });

    child.on('exit', (code, signal) => {
      if (signal) {
        process.kill(process.pid, signal);
      } else {
        process.exit(code || 0);
      }
    });
  } catch (err) {
    process.stderr.write(`Error launching agent-mem: ${err.message}\n`);
    process.exit(1);
  }
}

if (require.main === module) main();

module.exports = { ensureBinary, getTargetAsset };
