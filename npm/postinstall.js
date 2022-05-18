#!/usr/bin/env node

import { install, checkInstallation } from './index.js';

try {
  await checkInstallation();
  await install({ force: true });
} catch (error) {
  console.error(error);
  process.exit(1);
}
