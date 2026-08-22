Starline Defender
=================

Starline Defender is an original deterministic static-web plane shooter used by
the AIT agent-token benchmark. It has no backend, package dependency, hosted
asset, or network requirement.

Run
---

    npm run serve

Then open http://127.0.0.1:4173/?seed=1337&bossAfter=10000.

Validate
--------

    npm test

Node.js is used only for the project-local static server and deterministic
fixture checks. Python is neither required nor supported.
