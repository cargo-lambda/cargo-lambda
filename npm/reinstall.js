#!/usr/bin/env node

import { install } from './index.js';

try {
  await install({ force: true });
} catch (error) {
  console.error(error);
  process.exit(1);
}
