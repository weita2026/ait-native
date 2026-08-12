# ait-native

`ait-native` is the direct in-process Node-API binding for the Rust-owned AIT
runtime. JavaScript and TypeScript load a package-owned
`native/ait_napi.node`; the package does not launch an `ait` executable.

```js
import { NativeRuntime } from "ait-native";

const ait = new NativeRuntime();
console.log(ait.bindingInfo());
const status = ait.runCli(["status"]);
```

The package also installs one `ait` command backed by that same addon.
`ait-server` is distributed separately and is not part of npm. Runtime code
does not use install hooks, downloads, project-language detection, or
`child_process` transport.

The complete product, platform, source, and licensing contract is published
at <https://github.com/weita2026/ait-native/blob/v1.0.0-rc.2/docs/distribution.md>.
