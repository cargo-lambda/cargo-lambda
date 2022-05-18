#!/usr/bin/env node

import * as fs from 'fs/promises';
import util from 'util';
import { exec } from 'child_process';
import { exit } from 'process';
const execAsync = util.promisify(exec);

try {
    await fs.rm('bin', { recursive: true });
    await execAsync('git checkout bin');
} catch (error) {
    console.log(error);
    exit(1);
}
