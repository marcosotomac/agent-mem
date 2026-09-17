#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const os = require('os');
const { spawn, execSync } = require('child_process');
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

function findSystemBinary() {
  const isWin = os.platform() === 'win32';

  // 1. Check local build in development
  const localDevBin = path.join(__dirname, '..', '..', 'target', 'release', isWin ? 'agent-mem.exe' : 'agent-mem');
  if (fs.existsSync(localDevBin)) {
    return localDevBin;
  }

  // 2. Check system PATH
  const cmd = isWin ? 'where agent-mem.exe' : 'which agent-mem';
  try {
    const out = execSync(cmd, { stdio: ['ignore', 'pipe', 'ignore'] }).toString().trim();
    if (out && fs.existsSync(out)) {
      return out;
    }
  } catch (_) {}

  return null;
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

async function verifyArchive(archivePath, asset, releaseBaseUrl) {
  const checksumsPath = `${archivePath}.sha256sums`;
  await download(`${releaseBaseUrl}/SHA256SUMS.txt`, checksumsPath);
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

async function ensureBinary() {
  const sysBin = findSystemBinary();
  if (sysBin) return sysBin;

  const asset = getTargetAsset();
  const isWin = os.platform() === 'win32';
  const binName = isWin ? 'agent-mem.exe' : 'agent-mem';
  const cacheDir = path.join(os.homedir(), '.cache', 'agent-mem', `v${VERSION}`);
  const binPath = path.join(cacheDir, binName);

  if (fs.existsSync(binPath)) {
    return binPath;
  }

  fs.mkdirSync(cacheDir, { recursive: true });
  const archivePath = path.join(cacheDir, asset);
  const releaseBaseUrl = `https://github.com/${REPO}/releases/download/v${VERSION}`;
  const downloadUrl = `${releaseBaseUrl}/${asset}`;

  // Log to stderr only so stdout JSON-RPC MCP channel is NEVER polluted
  process.stderr.write(`[agent-mem] Downloading native binary v${VERSION} for ${os.platform()}-${os.arch()}...\n`);

  await download(downloadUrl, archivePath);
  await verifyArchive(archivePath, asset, releaseBaseUrl);

  if (asset.endsWith('.tar.gz')) {
    execSync(`tar -xzf "${archivePath}" -C "${cacheDir}"`, { stdio: 'ignore' });
  } else if (asset.endsWith('.zip')) {
    if (isWin) {
      execSync(`powershell -Command "Expand-Archive -Path '${archivePath}' -DestinationPath '${cacheDir}' -Force"`, { stdio: 'ignore' });
    } else {
      execSync(`unzip -o "${archivePath}" -d "${cacheDir}"`, { stdio: 'ignore' });
    }
  }

  try {
    fs.unlinkSync(archivePath);
  } catch (_) {}

  if (!isWin) {
    fs.chmodSync(binPath, 0o755);
  }

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

main();
