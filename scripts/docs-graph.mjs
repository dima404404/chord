#!/usr/bin/env node
// @ts-nocheck
/**
 * scripts/docs-graph.mjs — Automated Project Knowledge Graph & Map Generator
 *
 * Scans workspace architecture, Rust crates, UI features, and AMCP v2.0 bus,
 * validates Mermaid syntax, and synchronizes the master graph in docs/map.md.
 *
 * Usage:
 *   node scripts/docs-graph.mjs [--check]
 */

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { generateArchitectureMermaid } from './sync-project-map.mjs';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const PROJECT_ROOT = path.resolve(__dirname, '..');
const MAP_FILE = path.join(PROJECT_ROOT, 'docs/map.md');

export function updateMapFile(checkOnly = false) {
  if (!fs.existsSync(MAP_FILE)) {
    console.error(`❌ Map file not found at ${MAP_FILE}`);
    process.exit(1);
  }

  const content = fs.readFileSync(MAP_FILE, 'utf8');
  const graphContent = generateArchitectureMermaid();

  // Replace existing first mermaid block
  const mermaidRegex = /```mermaid[\s\S]*?```/;
  if (!mermaidRegex.test(content)) {
    console.error('❌ Could not find ```mermaid block in docs/map.md');
    process.exit(1);
  }

  const updatedContent = content.replace(mermaidRegex, graphContent);

  if (checkOnly) {
    if (content === updatedContent) {
      console.log('✅ [DOCS-GRAPH] docs/map.md graph is 100% up to date.');
      return true;
    } else {
      console.log('⚠️  [DOCS-GRAPH] docs/map.md graph is out of date. Run npm run docs:graph to update.');
      return false;
    }
  }

  fs.writeFileSync(MAP_FILE, updatedContent, 'utf8');
  console.log('✅ [DOCS-GRAPH] Successfully regenerated and synchronized docs/map.md graph!');
  return true;
}

function main() {
  const checkOnly = process.argv.includes('--check');
  const success = updateMapFile(checkOnly);
  process.exit(success ? 0 : 1);
}

main();
