# MCP server (`dashcraft mcp`)

`dashcraft mcp` is a [Model Context Protocol](https://modelcontextprotocol.io/specification) server on stdin/stdout. It lets an AI agent (Claude Desktop, VS Code, GitHub Copilot CLI, any MCP client) open Power BI files offline, explore the model and reports, evaluate measures and DAX, and render report pages as images.

It is a thin adapter over the command registry ([ADR-0007](adr/0007-command-registry.md), [COMMANDS.md](COMMANDS.md)): every **agent-safe** command becomes a tool, with its parameter and result JSON Schemas, and every call runs through `Registry::execute` as an `Agent` caller inside a sandbox of read/write roots. There is no per-tool code, so new commands become tools automatically. The implementation is `pbi_ops::mcp` (hand-rolled JSON-RPC 2.0, the same framing as `dashcraft serve`).

```sh
dashcraft mcp --read-root ~/PowerBI                         # read-only, files under ~/PowerBI
dashcraft mcp --read-root ~/PowerBI --write-root ~/PowerBI/out --blob-map ~/PowerBI/blob-map.json
dashcraft mcp --read-root . --allow inspect --allow dax --allow workspace   # a smaller tool set
```

## Flags

| Flag | Meaning |
|---|---|
| `--read-root DIR` (repeatable) | Agents may read files inside `DIR`. Relative paths resolve against the first root. **Default: the current folder** (a one-line notice is printed on stderr), except the filesystem root, which is refused: clients often start servers in `/`, so always pass `--read-root` in client configurations. |
| `--write-root DIR` (repeatable) | Agents may write files inside `DIR` (also readable). **Without it every write is refused** (`pathDenied`); inline results (SVG, PNG, JSON) still work. |
| `--blob-map FILE` | Default blob map of `doc_open` (offline `AzureStorage.Blobs` URLs → local CSVs). The map and every file it maps must be inside the read roots. |
| `--allow CATEGORY\|ID` (repeatable) | Only expose these tools: a category (`workspace`, `inspect`, `lint`, `data`, `dax`, `render`, `canvas`, `export`, `convert`, `transform`, `meta`), a command id (`doc.open`) or a tool name (`doc_open`). |
| `--deny CATEGORY\|ID` (repeatable) | Never expose these (applied after `--allow`). An entry that matches nothing is an error, so typos don't silently widen the set. |
| `--tool-names underscore\|dotted` | Tool names `doc_open` (default) or `doc.open`. |
| `--max-concurrent N` | Tool calls running at the same time (default: CPU count, 2 to 8). |

Global `-v`/`-q` control the logs on stderr. stdout carries MCP messages only.

## Security model

- **Agent-safe commands only.** Tools are `Registry::agent_catalog()` (the **A** flag in [COMMANDS.md](COMMANDS.md)); calls run with `CallerKind::Agent`, so even `commands_batch` steps can only run exposed tools. `--allow`/`--deny` narrow the set further.
- **Read and write roots.** Every path a tool receives (documents, blob maps, CSVs, output files) goes through the workspace's `PathPolicy`: the existing part of the path is canonicalized and must stay inside a root, the missing part may only contain plain names, `..` and symlink escapes are refused, writing through a symlink is refused, and blob maps may only map files inside the roots. The server refuses to start without a restricted policy. Limit: the check happens before the file is opened (TOCTOU), so roots should be folders only the user controls.
- **No network.** No tool talks to the network (`openWorldHint: false` on every tool). Blob URLs resolve through the local blob map only; other M data sources return "unsupported".
- **No code execution.** Embedded scripts, R/Python visuals, custom visuals (`.pbiviz`) and connectors (`.mez`/`.pqx`) are never executed, only listed. DAX and M are evaluated by DashCraft's own engines.
- **Bounded resources.** At most 32 open documents per session, a bounded number of concurrent and pending calls (`busy` beyond), 16 MiB per message line, and tool result text capped at 1 MiB.
- **Session state.** Documents opened with `doc_open` stay open for the life of the process (one workspace per server); `doc_close` frees them. End of input cancels the running calls and exits.

## Tools

`tools/list` derives each tool from the command's metadata:

| MCP field | Source |
|---|---|
| `name` | The command id with `.` → `_` (`measures.values` → `measures_values`); `--tool-names dotted` keeps the id. Names match `^[a-zA-Z0-9_-]{1,64}$`, the mapping is reversible and checked for collisions at startup. |
| `title`, `description` | `CommandMeta.title`; `CommandMeta.summary` plus the first example's arguments |
| `inputSchema` | The parameter JSON Schema (draft 2020-12, `type: object`, `additionalProperties: false`) |
| `outputSchema` | The result JSON Schema (when it is an object schema) |
| `annotations` | From the command flags (below); `openWorldHint` is always `false` |
| `_meta` | `dashcraft/commandId` (the dotted id) and `dashcraft/category` |

| Hint | Rule |
|---|---|
| `readOnlyHint` | Neither mutates a document nor writes files |
| `destructiveHint` | Writes files (may overwrite with `overwrite: true`); in-memory document changes are session-only and not destructive |
| `idempotentHint` | Does not mutate a document (clicks, scrolls, CSV loads are not idempotent) |

`commands_batch` runs other tools, so its hints are the union of the exposed tools'.

Tool results:

- **Success**: `content` is the result as JSON text and `structuredContent` the same JSON (validated by the output schema). `"ok": false` results (⚠ measures, failed visuals) are still successes with data.
- **Images**: an inline PNG render (`render_page`/`render_visual` with `"format": "png"`, or a batch step doing so) is returned as `image` content (`mimeType: image/png`); the base64 is moved out of the text and `structuredContent` (`pngBase64` is optional in the schema), so the text stays small. SVG renders stay text.
- **Large results**: text over 1 MiB is cut with a note naming the tool's paging parameters (`limit`, `offset`, `filter`, `outputPath`, …) and marked `isError` (no `structuredContent`).
- **Command errors** (bad arguments, `pathDenied`, `noDocument`, DAX errors, …) are results with `isError: true` and the error as JSON text, `{"code": "<string code>", "message": "…", "data"?: …}`, so the model can correct itself. Codes are listed in [COMMANDS.md](COMMANDS.md#error-codes).
- **Protocol errors** are JSON-RPC errors: unknown tool, missing `name`, `arguments` that are not an object, invalid cursor → `-32602`; unknown method → `-32601`; malformed JSON → `-32700`; invalid message → `-32600`.

## Protocol coverage

Newline-delimited JSON-RPC 2.0 over stdio (no batches).

| Message | Support |
|---|---|
| `initialize` | Revisions `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`: the client's revision when served, else `2025-11-25`. Capabilities `tools: {listChanged: false}` and `logging: {}`; `serverInfo`, `instructions` (how to start, and the roots). Older revisions get older shapes (no `title`/`outputSchema`/`structuredContent` before `2025-06-18`, no `annotations` before `2025-03-26`). |
| `notifications/initialized` | Accepted |
| `ping` | `{}` |
| `tools/list` | Deterministic order, cursor pagination (100 per page) |
| `tools/call` | Runs concurrently; mutating calls on the same `docId` run in arrival order |
| `notifications/cancelled` | Cancels the call's `CancellationToken`; no response follows (as the spec requires) |
| `notifications/progress` | Sent for calls with `_meta.progressToken`: `progress` 0–100 (`total: 100`), strictly increasing, with the command's message (refresh steps, rows per table) |
| `logging/setLevel`, `notifications/message` | After `setLevel`: `debug` per finished call, `warning`/`error` per failed call (logger `dashcraft`) |
| `2026-07-28` (stateless) | `server/discover`; requests carrying `_meta["io.modelcontextprotocol/protocolVersion"]` are served statelessly (`resultType: "complete"`, `_meta` server info, `ttlMs`/`cacheScope` on `tools/list`, per-request `io.modelcontextprotocol/logLevel`); an unknown version gets `-32022` with the supported list |

Not offered: resources, prompts, completions, sampling, elicitation, tasks.

## Client configuration

Use the absolute path of the `dashcraft` binary if it is not on the client's `PATH`. Give the narrowest roots that contain your files.

**Claude Desktop** (`claude_desktop_config.json`; macOS: `~/Library/Application Support/Claude/`, Windows: `%APPDATA%\Claude\`):

```json
{
  "mcpServers": {
    "dashcraft": {
      "command": "/usr/local/bin/dashcraft",
      "args": ["mcp", "--read-root", "/Users/me/PowerBI", "--blob-map", "/Users/me/PowerBI/blob-map.json"]
    }
  }
}
```

**VS Code** (`.vscode/mcp.json` in the workspace):

```json
{
  "servers": {
    "dashcraft": {
      "type": "stdio",
      "command": "dashcraft",
      "args": ["mcp", "--read-root", "${workspaceFolder}", "--write-root", "${workspaceFolder}/out"]
    }
  }
}
```

**GitHub Copilot CLI** (`~/.copilot/mcp-config.json`, or `/mcp add` in the CLI):

```json
{
  "mcpServers": {
    "dashcraft": {
      "type": "local",
      "command": "dashcraft",
      "args": ["mcp", "--read-root", "/Users/me/PowerBI"],
      "tools": ["*"]
    }
  }
}
```

Clients that accept dotted names can use `"--tool-names", "dotted"` to see the command ids (`doc.open`).

## Transcript

`→` is the client (stdin), `←` the server (stdout); one message per line, shortened here with `…`. The project is `Sales.pbip` under the read root.

```text
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"demo","version":"1"}}}
← {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{"listChanged":false},"logging":{}},"serverInfo":{"name":"dashcraft","version":"0.2.0","title":"DashCraft: Power BI files, offline"},"instructions":"Reads Power BI files (PBIP, PBIX, PBIT, TMDL, …) offline: no network, no script execution. Start with `doc_open` {\"path\": …} … Readable folders: /Users/me/PowerBI. Writable folders: none."}}
→ {"jsonrpc":"2.0","method":"notifications/initialized"}
→ {"jsonrpc":"2.0","id":2,"method":"tools/list"}
← {"jsonrpc":"2.0","id":2,"result":{"tools":[…,{"name":"measures_values","title":"Measure values","description":"Every measure's formatted value under optional report/page filters; failed measures show ⚠ and their DAX. Example arguments: {\"docId\": \"d1\"}","inputSchema":{"type":"object","additionalProperties":false,"properties":{"docId":{"type":"string",…},"report":{"type":["string","null"],…},"page":{…},"filters":{…}},"required":["docId"],…},"outputSchema":{"type":"object",…},"annotations":{"title":"Measure values","readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false},"_meta":{"dashcraft/commandId":"measures.values","dashcraft/category":"dax"}},…]}}
→ {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"doc_open","arguments":{"path":"Sales.pbip","blobMap":"blob-map.json"}}}
← {"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"{\"docId\":\"d1\",\"path\":\"Sales.pbip\",\"format\":\"pbip\",…,\"tables\":3,\"measures\":5,\"reports\":[{\"name\":\"Sales\",\"pages\":2}],…}"}],"isError":false,"structuredContent":{"docId":"d1","path":"Sales.pbip","format":"pbip",…}}}
→ {"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"measures_values","arguments":{"docId":"d1"},"_meta":{"progressToken":"m1"}}}
← {"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":"m1","progress":0.0,"total":100,"message":"loading the data of d1 (first use)"}}
← {"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":"m1","progress":1.0,"total":100,"message":"refreshing 3 table(s)"}}
← {"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":"m1","progress":33.33,"total":100,"message":"Orders: 1200 rows in 41 ms"}}
← …
← {"jsonrpc":"2.0","method":"notifications/progress","params":{"progressToken":"m1","progress":100.0,"total":100,"message":"refresh finished in 95 ms"}}
← {"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"{\"ok\":true,…,\"measures\":[{\"table\":\"Orders\",\"name\":\"Total Sales\",\"value\":48210,\"formatted\":\"48210\",\"display\":\"48210\",…},…]}"}],"isError":false,"structuredContent":{"ok":true,…,"measures":[{"table":"Orders","name":"Total Sales","value":48210,"formatted":"48210",…},…]}}}
→ {"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"doc_open","arguments":{"path":"/etc/passwd"}}}
← {"jsonrpc":"2.0","id":5,"result":{"content":[{"type":"text","text":"{\"code\":\"pathDenied\",\"message\":\"read /etc/passwd: outside the allowed read roots\"}"}],"isError":true}}
```

Tool calls run concurrently: wait for `doc_open`'s response before calling a tool with its `docId`.
