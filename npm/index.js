#!/usr/bin/env node

import * as fs from 'fs/promises';
import * as os from 'os';
import * as path from 'path';
import * as url from 'url';
import download from 'download';
import which from 'which';
import util from 'util';
import { exec, spawn } from 'child_process';
const execAsync = util.promisify(exec);
const spawnAsync = util.promisify(spawn);

const __dirname = path.dirname(url.fileURLToPath(import.meta.url));

const platforms = {
  'Linux x64': 'x86_64-unknown-linux-musl.tar.gz',
  'Linux arm64': 'aarch64-unknown-linux-musl.tar.gz',
  'Linux x32': 'i686-unknown-linux-musl.tar.gz',
  'Windows_NT x64': 'x86_64-pc-windows-msvc.zip',
  'Windows_NT x32': 'i686-pc-windows-msvc.zip',
  'Darwin x64': 'apple-darwin.tar.gz',
  'Darwin arm64': 'apple-darwin.tar.gz'
};

const windows = os.type() === 'Windows_NT';
const installDirectory = path.join(__dirname, 'bin');
const name = windows ? 'cargo-lambda.exe' : 'cargo-lambda';
const binaryPath = path.join(installDirectory, name);

export async function install({ force = false } = {}) {
  if (!force) {
    try {
      // an empty ./bin/cargo-lambda file is used as a placeholder for npm/pnpm/yarn to
      // create the bin symlink, so the file exists but will have a zero size in
      // the base case, check for it here
      const stats = await fs.lstat(binaryPath);
      if (stats.isSymbolicLink()) {
        console.log(`Replacing symlink with local installation: ${binaryPath}`);
      } else if (stats.size !== 0) {
        console.log(`${name} is already installed, did you mean to reinstall?`);
        return;
      }
    } catch {}
  }

  await uninstall();
  await fs.mkdir(installDirectory, { recursive: true });

  const data = await fs.readFile('package.json', 'utf8');
  const version = JSON.parse(data).version

  const type = `${os.type()} ${os.arch()}`;
  const platform = platforms[type];
  if (!platform) {
    throw new Error(`Unsupported platform ${type}. See other installation options in the homepage: https://github.com/cargo-lambda/cargo-lambda#installation`);
  }

  const url = `https://github.com/cargo-lambda/cargo-lambda/releases/download/v${version}/cargo-lambda-v${version}-${platform}`;
  await download(url, installDirectory, { extract: true });
}

export async function run(...args) {
  try {
    await fs.access(binaryPath);
  } catch (err) {
    throw new Error(`You must install ${name} before you can run it`);
  }

  await spawnAsync(binaryPath, args, { stdio: 'inherit' });
}

export async function uninstall() {
  await fs.rm(installDirectory, { recursive: true });
}

export async function checkInstallation() {
  try {
    const stats = await fs.lstat(binaryPath);
    // if the binary is a symlink, it should link to the system-wide cargo-lambda
    if (stats.isSymbolicLink()) {
      console.log('Skipping cargo-lambda installation, symlink to system cargo-lambda exists');
      process.exit(0);
    }
    // an empty ./bin/cargo-lambda file is used as a placeholder for npm/pnpm/yarn to
    // create the bin symlink, so if the file already exists and has non-zero
    // size, and is not a symlink, then cargo-lambda is already installed locally
    if (stats.size !== 0) {
      console.log('Skipping cargo-lambda installation, binary exists');
      process.exit(0);
    }
  } catch {}
  try {
    // remove the node_modules/.bin symlinks from the PATH before checking if a
    // cargo-lambda is installed system-wide, to avoid hitting the symlink
    const pathSep = windows ? ';' : ':';
    const pathDirs = process.env.PATH.split(pathSep);
    process.env.PATH = pathDirs
      .filter(dir => !dir.endsWith(path.join('node_modules', '.bin')))
      .join(':');
    // check if there's already an installed cargo-lambda binary
    const systemVersion = await which('cargo-lambda');

    const lambdaVersion = await execAsync('cargo-lambda lambda --version');
    const systemLambdaVersion = lambdaVersion.stdout.trim();

    if (systemVersion.length !== 0 && systemLambdaVersion.length !== 0) {
      console.log(
        `Skipping cargo-lambda installation, ${systemLambdaVersion} already installed in system`,
      );
      console.log(`Creating symlink to: ${systemVersion}`);
      await fs.unlink(binaryPath);
      await fs.link(systemVersion, binaryPath);
      console.log(
        `Manually run the cargo-lambda-install script to install it locally`,
      );
      process.exit(0);
    }
  } catch (error) {}
  console.log('Did not detect cargo-lambda in system, will be installed locally');
}
