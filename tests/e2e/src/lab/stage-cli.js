#!/usr/bin/env node

import { readFileSync } from 'node:fs';

import { writeStage } from './stage.js';

const [dir, remoteHome] = process.argv.slice(2);
if (!dir || !remoteHome) {
  console.error('usage: stage-cli.js <dir> <remote lab root>');
  process.exit(2);
}
const releaseServer = readFileSync(new URL('../../lab/release-server.mjs', import.meta.url), 'utf8');
writeStage(dir, { remoteHome, releaseServer });
