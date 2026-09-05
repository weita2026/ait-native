# ait-native

[English](README.md) · [简体中文](README_CN.md) · **[繁體中文](README_ZH.md)**

**將多個並行的 coding agent 對話，變成經過驗證、可追溯的 Task。**

AIT 是以本地工作為主的 CLI，適合將程式修改交給 agent、並對結果負責的個人開發者與維護者。
它將每個需求連結到獨立 worktree、通過檢查的確切版本，以及發生回歸時需要的歷史紀錄。
使用你自己的 coding agent，由你掌握任務意圖與驗收。

[![最新穩定版本](https://img.shields.io/github/v/release/weita2026/ait-native?label=stable)](https://github.com/weita2026/ait-native/releases)
[![文件](https://img.shields.io/badge/docs-ait--native.dev-0ea5e9)](https://ait-native.dev/)
[![授權](https://img.shields.io/badge/license-Apache--2.0%20%2B%20AGPL--3.0--only-22c55e)](#license-map)

**[試跑第一個 Task](https://ait-native.dev/zh-tw/local-quickstart/#first-task)** ·
[觀看演示](https://ait-native.dev/zh-tw/demo/#in-action) ·
[技術文件](https://ait-native.dev/technical/) ·
[取得協助](https://github.com/weita2026/ait-native/discussions)

[简体中文入门](https://ait-native.dev/zh-cn/local-quickstart/#first-task) ·
[繁體中文入門](https://ait-native.dev/zh-tw/local-quickstart/#first-task)

## 走完一個 AIT 任務

[![AIT Task 實際錄製：初始化、開始隔離工作、測試、完成並追溯結果](https://ait-native.dev/public/tour/ait-task-tour.gif)](https://ait-native.dev/zh-tw/demo/#in-action)

這是 AIT 1.1.1 實際命令輸出的錄製，剪輯成 33 秒的重播，展示一個 Task 如何完成下方可下載的同一個範例。
路徑已簡化；播放時間不是效能測量。
[觀看附字幕與完整文字紀錄的版本](https://ait-native.dev/zh-tw/demo/#in-action)。

若要了解多個 Task，請查看另外提供的
[並行工作與回歸情境示意](https://ait-native.dev/zh-tw/demo/#demo-scene)，
或[以繁體中文標示的工作流說明圖](https://ait-native.dev/public/tour/ait-workflow-zh-tw.png)。

## 試跑第一個 Task

請從全新的範例資料夾開始。你需要自己的 coding agent，以及執行此小範例所需的 Node.js 22+；
Node.js 並非 AIT 對儲存庫的通用要求。

**1. 安裝 AIT。**

```sh
python -m pip install ait-native==1.1.1
ait --version
```

其他套件通路請參閱[安裝指南](https://ait-native.dev/zh-tw/local-quickstart/)。

**2. 準備範例。**

[下載範例 ZIP](https://ait-native.dev/downloads/ait-first-task.zip)
及[它的 SHA-256](https://ait-native.dev/downloads/ait-first-task.sha256)。
解壓縮後，在解壓得到的 `ait-first-task` 資料夾開啟終端。
將 `your-name` 換成你希望記錄於本地審查的名稱。

```sh
node --test tests/baseline.test.mjs
ait init
ait config set --user-name "your-name"
ait snapshot create --message "Start the AIT example"
```

三項基準測試應全部通過。這個初始 Snapshot 會在 Task 開始前記錄尚未修改的範例。
若使用既有專案，請改依[入門指南](https://ait-native.dev/technical/getting-started/)操作。

**3. 在 coding agent 中開啟範例，並給它以下需求。**

> 閱讀 AGENTS.md，遵循此儲存庫的 AIT 工作流。在 src/tasks.mjs 新增 openTasks(tasks)：
> 回傳新的陣列，包含 done 屬性不等於 true 的任務，保留原有順序、任務物件及輸入資料。
> 保持 taskTitles 正常運作，加入針對性的測試。不要修改 checks/、移除既有測試或新增相依套件。
> 完成 Task 前，執行既有測試及 node checks/first-task.mjs。

Agent 會記錄 sprint 項目，在回傳的 Task worktree 中實作修改並執行檢查。
產生的 `AGENTS.md` 區塊會提供該儲存庫適用的確切命令與完成要求。

**4. 回到原始範例資料夾檢查結果。**

```sh
node --test tests/*.test.mjs
node checks/first-task.mjs
```

功能檢查會輸出 `FIRST_TASK_ACCEPTED`。也請確認實際的 Task finish 結果：Task 已完成、
worktree 已清理，並在適用時關閉綁定的 sprint 項目。功能驗收與工作流完成是兩項分開的檢查。
審閱修改後，再到自己的專案嘗試一個小任務。

## 你能得到什麼

- **讓需求與工作持續相連。** 明確的 Markdown sprint 項目及其 Plan 版本會綁定至 Task。
  上下文壓縮後，產生的指示要求 agent 重新閱讀該項目。
- **以專案目前的狀態完成工作。** 獨立 Task 各自擁有 worktree。AIT 會重新檢查目標，
  對相容修改進行 rebase，遇到實質衝突則停止，並在 Task finish 成功後清理工作區。
- **出問題時找到相關脈絡。** `ait blame` 將受影響的程式碼或 Plan 文字連結到已記錄的版本與可用工作流歷史。
  Agent 可據此診斷問題、做出範圍明確的修正，並驗證結果。

## 除了 worktree，為什麼還需要 AIT？

Worktree 為任務提供獨立檔案。AIT 則以同一個 Task 生命週期管理需求、版本、適用檢查與完成狀態。
請和你現有的 agent、Git、issue tracker、CI 及腳本一起評估：

| 你需要回答的問題 | AIT 記錄或協調的內容 |
| --- | --- |
| 這個修改對應哪個需求與驗收條件？ | 綁定至 Task 的確切 Plan 項目與版本。 |
| 哪個結果通過檢查？它能否整合至今天的目標？ | 版本身分、適用證據、目標檢查與可復原的完成狀態。 |
| 後來出現的回歸從哪裡開始？ | 可透過 blame 取得的版本與工作流脈絡。未知來源仍標示為未知。 |

實作與驗證可以並行；套用至共用目標 Line 前，准入會重新驗證並依序處理。
乾淨的 rebase 仍需要相關驗證。Worktree 指示是工作流約束，並非作業系統沙箱；
blame 提供來源紀錄，不會自動診斷或修復問題。

試用[相同的工作流情境](https://ait-native.dev/zh-tw/demo/)，比較確認意圖、審查、整合及調查問題所花的時間。

## 與 Git worktree 的測量比較

兩輪分別公開的 200-session 實驗，在相同的五個固定遊戲開發測試專案上使用 GPT-5.6 Sol 的 max 推理設定。
每種工作負載各有 20 組通過准入的配對嘗試。每輪有效結果包含 100 個全新 AIT session 與 100 個全新 Git session，
兩組的功能驗收結果均為 100/100。

| 實驗 | 工作流 | 有效 session | 各工作負載 token 節省比例的中位數（95% CI） | 各工作負載耗時節省比例的中位數 |
| --- | --- | ---: | ---: | ---: |
| 已發布的 1.1.0 基準 | Sprint 關閉 | 100 AIT + 100 Git | **34.95%** (27.85%-39.77%) | **21.04%** |
| 自然檢視重複實驗 | Sprint 開啟 | 100 AIT + 100 Git | **36.28%** (28.26%-41.83%) | **15.22%** |

這些模型供應商 token 的工作負載中位數結果，僅適用於指定模型與固定測試資料。
Session 是依序執行的，因此結果不衡量並行吞吐量，也不保證你的儲存庫會得到相同節省。

<details>
<summary>方法、排除項目與尚未完成的 Claude Fable 實驗</summary>

已發布基準的 AIT 使用 46,300,272 tokens，Git 使用 70,140,925 tokens（減少 33.99%）；
證據歷史包含 201 次執行，其中一筆功能結果被排除。
Sprint 開啟的重複實驗分別使用 45,432,262 與 71,238,660 tokens（減少 36.23%）；
證據歷史包含 203 次執行，並揭露三筆排除項目。
表中的工作負載平衡指標是主要測量值；合併總量僅供描述。

兩輪實驗共用相同測試資料位元組、工作負載矩陣、固定模型，以及對稱的唯讀檢視權限。
它們的工作流模式、提示詞、受測 AIT 執行檔、隨機種子、日期及復原歷史不同。
這是結果方向相近的重複實驗，不是合併觀察值，也不是能推論 sprint 開關因果效果的 A/B 測試，
因此不能將 1.33 個百分點的差異歸因於 sprint 模式。
結果範圍仍限於這些固定測試資料與依序執行的全新 session，不能承諾適用所有工作負載，也未測量高併發執行。

[已發布基準的證據](https://github.com/weita2026/ait-native/tree/v1.1.0/ait-core/release/benchmarks/game-v1-g56s-max-complete200-fx27-20260826) ·
[Sprint 開啟重複實驗的證據](https://github.com/weita2026/ait-native/tree/benchmark-sprint-on-20260829/ait-core/release/benchmarks/game-v1-g56s-max-sprint-on-natural-complete200-20260828)

### Claude Fable 評測——仍在進行

我們正在執行一輪設定已固定的 200-session 評測，比較 AIT 的任務導向工作流與 agent 自行管理的本地 Git worktree，
兩組使用相同的五種遊戲開發工作負載。

**進度：22 / 200 個 session**

目前觀察到的 22 次執行全部有效且通過驗收，沒有切換至備用模型。
實驗尚未完成，仍為 `claim_eligible=false`。
無論剩餘結果偏向 AIT 或 Git，我們都會繼續完成全部 200 個 session。

最新的平衡檢查點為 20/200，每種工作負載各有兩組完整 AIT/Git 配對：

| 工作負載 | 有效配對 | AIT token 節省比例 | Bootstrap CI95 |
| --- | ---: | ---: | ---: |
| GD-01 | 2 | 20.32% | 9.13% to 32.72% |
| GD-02 | 2 | -4.35% | -18.16% to 14.35% |
| GD-03 | 2 | 37.56% | 6.57% to 52.02% |
| GD-04 | 2 | 23.77% | 23.50% to 23.97% |
| GD-05 | 2 | 2.13% | -26.89% to 25.93% |

各工作負載 token 節省比例的中位數為 **20.32%**，整體 bootstrap CI95 為 **6.57% to 25.93%**。
此檢查點的全部 20 次執行均有效且通過驗收，沒有統計排除，也沒有切換至備用模型。
每種工作負載僅有兩組配對，部分區間仍很寬或跨越零，因此公布這些中期數字是為了透明揭露，並非產品成效宣稱。

</details>

## 我為什麼打造 AIT

<details>
<summary>促使我打造 AIT 的六個問題</summary>

1. **AI agent 經常產出一個巨大的 commit，卻很難看出它代表什麼工作。**

   Agent 可能修改數十個檔案，再將所有內容塞進同一個 commit。
   Commit 看得出改了什麼，卻不清楚說明 agent 想完成哪件工作。
   我希望歷史紀錄圍繞有意義的任務組織，而非圍繞 agent 剛好存下工作的時點。

2. **Sprint 卡應該對應到真正的工程工作。**

   我想要類似 Jira 的工作流：開一張 sprint 卡，就啟動一個真正隔離的任務；
   完成任務，應代表問題確實獲得解決。Ticket、agent、程式碼、驗證與最終結果，應屬於同一個生命週期。

3. **傳統 Git 工作流是圍繞人的行為設計的。**

   人通常先做一個小修改，再審查、暫存、commit、rebase，然後繼續下一步。
   在 vibe coding 時代，agent 產出任務規模修改的速度更快。
   每次都替每個 agent 重複這些手動 Git 操作，開始成為工作阻力。

4. **Markdown 應該不只是儲存庫裡的另一個檔案。**

   Markdown 可能是人與 agent 最好的共同語言。Git 可以儲存 Markdown，
   卻不理解一個清單項目代表計劃、任務或驗收條件。
   我希望寫在 Markdown 裡的意圖，能持續連結到實現它的程式碼。

5. **當 agent 弄壞東西時，我希望很快得到答案。**

   我不想為了理解一個回歸，翻找舊對話、零散 commit 與互不相連的 ticket。
   AIT 將任務、版本、驗證、agent 脈絡與 Task finish 歷史連在一起，
   讓 `ait blame` 能從出錯的程式碼行追溯至引入問題的工作。

6. **命令優先為 agent 設計。**

   CLI 的設計重點，不是讓人反覆輸入時覺得順手，而是讓 agent 難以誤解：
   穩定的命令、明確的狀態、結構化結果、確切的工作區、清楚的失敗訊息，以及明確的下一步。
   人仍然決定意圖、審閱結果，並承擔後果。

</details>

## 搭配現有工具使用

AIT 不限定儲存庫使用哪種語言，也不嘗試偵測專案類型；建置、測試與忽略規則都來自你的儲存庫。
Coding agent 負責實作及選定的檢查；AIT 管理任務生命週期，並執行適用的工作流條件。

`ait init` 建立本地 `.ait` 權威資料，並產生儲存庫的 `AGENTS.md` 工作流區塊。
該產生式區塊是有效命令的依據；本地工作不需要執行中的 `ait-server`。

AIT 有兩種工作流預設模式：`solo_local` 將工作與 Task finish 保留在本地；
`solo_remote` 加入明確選定的伺服器與經審查的完成流程。
Agent 依產生的指示執行 `ait task start`、以 `ait snapshot create` 保存中途檢查點、
以 `ait plan sync` 保存 Markdown 沿革，並使用適用的 `ait task finish` 或 `ait workflow finish` 收尾。

- [Git 匯入、匯出與退出路徑](https://ait-native.dev/technical/cli/reference/git/)
- [功能工作流](https://ait-native.dev/technical/workflows/feature/)與[回歸修復](https://ait-native.dev/technical/workflows/regression/)
- [元件](https://ait-native.dev/zh-tw/components/)與[發行狀態](https://ait-native.dev/zh-tw/proof/)

目前公開版本：**v1.1.1**。如需該版本的確切原始碼，請使用不可變的發行標籤；
`ait-monorepo-source.json` 記錄元件 Snapshot 映射。兩次發行之間，`main` 的文件可能繼續更新。

## 各安裝通路提供什麼

<details>
<summary>套件內容與歷史版本差異</summary>

| 通路 | 安裝內容 |
| --- | --- |
| PyPI `ait-native` | `ait`、預設不啟動的 `ait-server`，以及直接 `ait-python` 綁定。 |
| npm `@wa120/ait-native` | `ait` 及直接在程序內運作的 Node-API 綁定；不安裝 `ait-server`。 |
| Homebrew 與 WinGet | 1.1.1 產品組合包含原生 `ait`、`ait-server` 與 `ait-runner`。安裝時不啟動任何背景程序；通路可用性請查閱發行狀態。 |
| APT | 在 1.1.1，`ait-native` 擁有全部三個命令；`ait-runner` 是僅宣告相依性的過渡別名。套件內的服務仍僅適用伺服器。 |
| OCI | 分開提供 `ait-server` 與 `ait-runner` 映像。 |
| GitHub Release | 綁定校驗碼的原生壓縮檔，以及各宣告通路使用的套件資產。 |

不可變的 1.1.0 Homebrew、apt 與 WinGet 產品套件包含 `ait`／`ait-server` 組合，apt 另外提供 runner。
這項歷史例外保持不變。現行通路操作請參閱[安裝指南](https://ait-native.dev/zh-tw/local-quickstart/)，
確切資產請查閱[發行頁面](https://github.com/weita2026/ait-native/releases)。

</details>

## 從 0.x 升級

1.x 沒有 `ait install` 命令。請透過套件管理工具升級，並以 `ait --version` 確認版本。
保留既有 `.ait` 權威資料，遷移前先查看確切版本的轉換指示。
建立新的權威資料與升級既有歷史，是不同的操作。
請參閱[轉換契約](ait-core/docs/distribution.md#public-0x-to-10-transition)
與[Git 退出參考](https://ait-native.dev/technical/cli/reference/git/)。

## 建置這份原始碼

<details>
<summary>原生來源建置與語言綁定</summary>

在 macOS 或 Linux 的乾淨 checkout 中執行：

```sh
./build-release.sh
```

在 Windows PowerShell 中執行：

```powershell
.\build-release.ps1
```

建置會在 `dist/source-build/` 產生本地原生命令、直接 PyO3 Python wheel、
可攜的 JS/TS 套件，以及目前主機適用的直接 Node-API addon。
這些來源建置產物及其憑證明確標示為不可發布；受保護的發行 CI 只會提升另外通過准入的家族產物。

在 Node.js，`import { NativeRuntime, AgentClient } from "@wa120/ait-native"`
會在目前程序載入套件自有的 `native/ait_napi.node`。
npm 的 `ait` 命令透過 `NativeRuntime.runCli()` 呼叫同一個 Rust 綁定，不會尋找或啟動子執行檔。

</details>

## 分享結果或取得協助

[提問或分享工作流](https://github.com/weita2026/ait-native/discussions)，
或[回報問題](https://github.com/weita2026/ait-native/issues/new/choose)。
請告訴我們第一個 Task 是否完成、在哪裡需要協助，以及是否能在另一個任務重複完成此工作流。
只分享你可以公開的資訊；提供回饋不需要私有儲存庫。

<a id="license-map"></a>

## 授權對照

根目錄的 [`LICENSE`](LICENSE) 已明確說明：根層發行控制、文件、`ait-core/**`、
`ait-runner/**`、`ait-python/**` 與 `ait-node/**` 採用 Apache-2.0。
唯一的元件例外為 `ait-server/**`，採用 AGPL-3.0-only。
各元件子樹保留其確切的 `LICENSE` 與 `NOTICE`；綁附發行不會變更任一元件的授權。
公開 1.0 原始碼路徑不適用商業或專有授權。

完整的套件、原始碼、建置與授權契約位於 [`docs/distribution.md`](docs/distribution.md)。
