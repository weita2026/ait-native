#!/usr/bin/env node
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, renameSync, rmSync, existsSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { runReceiptTransaction, recoverReceiptTransaction } from './release_receipt_transaction.mjs';
const root = mkdtempSync(path.join(os.tmpdir(), 'ait-receipt-test-'));
try {
  const producer = path.join(root,'producer.mjs');
  writeFileSync(producer, `import fs from 'node:fs';const out=process.argv[2];fs.writeFileSync(out,'candidate=1.1.4;sha=exact');`);
  const output = path.join(root,'receipt.json');
  const action = {id:'a',cwd:root,argv:[process.execPath,producer,output],outputs:[output]};
  const context = {plan_sha256:'a'.repeat(64),release:{version:'1.1.4'},action,replacements:{}};
  assert.equal(runReceiptTransaction(root,'a',context,action,{},path.join(root,'log')).status,0);
  const journal = JSON.parse(readFileSync(path.join(root,'conductor-receipts/a.json')));
  // Simulate interruption after journal commit but before promotion/state commit.
  renameSync(output,journal.outputs[0].staged);
  assert.equal(recoverReceiptTransaction(root,'a',context),true);
  assert.equal(existsSync(output),true);
  // No conductor state at all: journal and candidate/plan/input hashes suffice.
  assert.equal(runReceiptTransaction(root,'a',context,action,{},path.join(root,'log')).status,0);
  assert.throws(()=>recoverReceiptTransaction(root,'a',{...context,plan_sha256:'b'.repeat(64)}),/drift/);
  writeFileSync(output,'wrong bytes');
  assert.throws(()=>recoverReceiptTransaction(root,'a',context),/output drift/);
  writeFileSync(output,'candidate=1.1.4;sha=exact');
  writeFileSync(producer,readFileSync(producer,'utf8')+'\n// changed');
  assert.throws(()=>recoverReceiptTransaction(root,'a',context),/drift/);
  const orphan=path.join(root,'orphan.json');writeFileSync(orphan,'untrusted');
  const orphanAction={...action,id:'orphan',outputs:[orphan],argv:[process.execPath,producer,orphan]};
  assert.equal(runReceiptTransaction(root,'orphan',{...context,action:orphanAction},orphanAction,{},path.join(root,'orphan.log')).status,65);
  // Failed producers never expose partial outputs; each attempt is preserved.
  const failing=path.join(root,'failing.mjs');writeFileSync(failing,`import fs from 'node:fs';fs.writeFileSync(process.argv[2],'partial');process.exit(75);`);
  const target=path.join(root,'recoverable.json');const failed={...action,id:'failed',argv:[process.execPath,failing,target],outputs:[target]};
  const ctx={...context,action:failed};
  assert.equal(runReceiptTransaction(root,'failed',ctx,failed,{},path.join(root,'failed.log')).status,75);
  assert.equal(existsSync(target),false);
  writeFileSync(failing,`import fs from 'node:fs';fs.writeFileSync(process.argv[2],'complete');`);
  assert.equal(runReceiptTransaction(root,'failed',ctx,failed,{},path.join(root,'failed.log')).status,0);
  // Probe-backed receipt adoption requires the exact independently checked bytes.
  const checked=path.join(root,'checked');writeFileSync(checked,'validated');
  const probe=path.join(root,'probe.mjs');writeFileSync(probe,`import fs from 'node:fs';process.exit(fs.readFileSync(process.argv[2],'utf8')==='validated'?0:65);`);
  const resume={...action,id:'resume',outputs:[checked],probe:{cwd:root,argv:[process.execPath,probe,checked]}};
  assert.equal(runReceiptTransaction(root,'resume',{...context,action:resume},resume,{},path.join(root,'resume.log')).status,0);
  console.log('receipt transaction fault tests passed');
} finally { rmSync(root,{recursive:true,force:true}); }
