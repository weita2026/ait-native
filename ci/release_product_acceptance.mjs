#!/usr/bin/env node
import { fileURLToPath } from "node:url";
import { realpathSync } from "node:fs";

// Map product scenarios to existing decisions. Source/fixture checks support
// these rows; they do not supply current-candidate host/Runner/agent results.
export const REQUIRED_PRODUCT_CHECKS = {
  "A120-01": "source_preflight/public_qualification: native Git-independent lifecycle",
  "A120-02": "public_qualification: four processes, conflict and shared Plan ownership",
  "A120-03": "public_qualification: finish races and original-receipt retry",
  "A120-04": "component_release/public_qualification: frozen package with later Local finish/rebase",
  "A120-05": "component_release/web_admission: sealed mode, restart and actual host roles",
  "A120-06": "component_release/public_qualification: accepted retry and cleanup/mapping separation",
  "A120-07": "public_qualification: Local mapping interruption and ordinary I/O boundaries",
  "A120-08": "public_qualification: actual Server transaction/Job publication failures",
  "A120-09": "clean_host_qualification: six native targets and four repository-language fixtures",
  "A120-10": "web_admission: actual registered agents, independent Reviewer and real Runner",
};

export function requireProductAcceptance(version) {
  const match = /^(\d+)\.(\d+)\./.exec(version ?? "");
  if (!match) throw new Error("product acceptance requires an exact release version");
  if (Number(match[1]) < 1 || (Number(match[1]) === 1 && Number(match[2]) < 2)) return;
  // The SOP assigns the operator verdict to the normal version card. No
  // executable A120 aggregate verifier exists, so even that verdict cannot
  // release this automatic hold or make legacy 32-row qualification suffice.
  throw new Error("Pre-tag operator HOLD: executable current-candidate A120-01..10 verification is unavailable. " +
    "Record Niels's verdict in the normal version coordination card; it does not clear this automatic hold. " +
    Object.entries(REQUIRED_PRODUCT_CHECKS).map(([id, check]) => `${id}: ${check}`).join("; ") + ". Gate H remains operator-owned.");
}

if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { requireProductAcceptance(process.argv[2]); }
  catch (error) { process.stderr.write(error.message + "\n"); process.exit(75); }
}
