// Regenerates src/lib/ops.gen.ts (typed wrappers over every command of the app's registry: the
// pbi-ops catalog plus the opt-in pbi-ai `ai.*` commands, `pbi_ai::catalog()`) from the
// registry's JSON Schemas. The generator is the `ops_typescript_is_up_to_date` test of
// crates/pbi-ai/tests/registry.rs (pbi-ops cannot depend on pbi-ai): run without --write
// (`npm run check:ops`) it fails when the committed file is stale, which is how CI (`cargo test`
// over the default members) catches a catalog change that was not regenerated. The app's own
// test `app_registry_is_the_generated_catalog` checks that the app runs exactly that catalog.
// Zero npm dependencies.
import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const write = process.argv.includes('--write')
const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const env = { ...process.env }
if (write) env.DASHCRAFT_BLESS = '1'
else delete env.DASHCRAFT_BLESS
const result = spawnSync('cargo', ['test', '-q', '-p', 'pbi-ai', '--test', 'registry', '--', 'ops_typescript_is_up_to_date', '--exact'], {
  cwd: root,
  env,
  stdio: 'inherit',
  shell: process.platform === 'win32',
})
process.exit(result.status ?? 1)
