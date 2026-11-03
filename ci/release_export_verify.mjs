#!/usr/bin/env node
// Legacy export receipts have no conductor journal. Recompute the deterministic
// export under the current authority, then compare content, bindings and versions.
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { digest } from './release_receipt_transaction.mjs';
const [native, prior, output] = process.argv.slice(2);
const json=file=>JSON.parse(readFileSync(file,'utf8'));
const receipt=json(path.join(output,'receipt.json'));
const version=json(path.join(native,'ait-release-family.json')).family.version;
if (receipt.contract!=='ait.release.source-export/v2'||receipt.status!=='pass'||receipt.version!==version||receipt.prior_version!==prior) throw new Error('legacy export release identity differs');
const temporary=mkdtempSync(path.join(path.dirname(output),'.verify-export-'));
try {
  const rebuilt=path.join(temporary,'rebuilt');
  const result=spawnSync(path.join(native,'ci/release_export_source.sh'),[native,prior,rebuilt],{encoding:'utf8',maxBuffer:64*1024*1024});
  if(result.status!==0) throw new Error(result.stderr||'fresh authority export failed');
  if(JSON.stringify(receipt)!==JSON.stringify(json(path.join(rebuilt,'receipt.json')))) throw new Error('legacy export candidate binding differs');
  for(const kind of ['final','prior']) for(const suffix of ['a','b']) {
    const relative=path.join(kind,`export-${suffix}`);
    if(digest(path.join(output,relative))!==digest(path.join(rebuilt,relative))) throw new Error(`legacy export content differs: ${relative}`);
  }
} finally {rmSync(temporary,{recursive:true,force:true});}
