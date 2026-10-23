<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import Modal from './Modal.svelte'
  import * as backend from '../lib/backend'
  import type { SecurityRole, ViewAsResult, ViewAsState } from '../lib/backend'
  import { plural } from '../lib/format'

  // Modeling → View as: Power BI Desktop's "View as roles" dialog. Check roles (a row is visible
  // if any checked role allows it) and / or "Other user" (what USERPRINCIPALNAME() returns, for
  // dynamic roles), then OK applies them to every query of the document (`security.viewAs`).
  // A role filter that cannot be evaluated is reported here and the previous view stays.
  let {
    roles,
    current,
    onclose,
    onapplied,
  }: {
    roles: SecurityRole[]
    /** The view in effect: its roles and user start checked. */
    current: ViewAsState
    onclose: () => void
    onapplied: (result: ViewAsResult) => void
  } = $props()

  const same = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: 'accent' }) === 0

  // The dialog edits a copy of the view in effect when it opened.
  const initial = untrack(() => ({ roles: roles.filter((r) => current.roles.some((n) => same(n, r.name))).map((r) => r.name), userName: current.userName ?? '' }))
  let checked = $state<string[]>(initial.roles)
  let otherUser = $state(initial.userName !== '')
  let userName = $state(initial.userName)
  let applying = $state(false)
  let error = $state<string | null>(null)
  let userInput = $state<HTMLInputElement | null>(null)

  const dynamicChecked = $derived(roles.filter((r) => r.dynamic && checked.includes(r.name)))
  const needsUser = $derived(otherUser && userName.trim() === '')

  function toggle(name: string, on: boolean) {
    checked = on ? [...checked.filter((n) => n !== name), name] : checked.filter((n) => n !== name)
    error = null
  }

  function toggleOther(on: boolean) {
    otherUser = on
    error = null
    if (on) queueMicrotask(() => userInput?.focus())
  }

  /** `Table: filter` lines of a role (its tooltip and summary). */
  function filters(role: SecurityRole): string[] {
    return role.tablePermissions.map((p) => `${p.table}: ${p.filterExpression?.trim() || '(no row filter)'}`)
  }

  async function apply() {
    if (applying || needsUser) return
    applying = true
    error = null
    // Model order, as Desktop lists them.
    const selected = roles.filter((r) => checked.includes(r.name)).map((r) => r.name)
    const result = await backend.viewAs(selected, otherUser ? userName : null)
    applying = false
    if (result.ok) onapplied(result.value)
    else error = result.error
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault()
      void apply()
    }
  }
</script>

<Modal title="View as roles" width={520} {onclose}>
  <p class="muted intro">
    See the report and the data as members of the checked roles would: each role's DAX row filters apply to every
    visual, table and measure (a row is visible if any checked role allows it). Nothing is saved to the file.
  </p>
  <ul class="roles" aria-label="Roles">
    {#each roles as role, i (i)}
      {@const lines = filters(role)}
      <li>
        <label class="check" title={[role.description ?? '', ...lines].filter(Boolean).join('\n')}>
          <input type="checkbox" checked={checked.includes(role.name)} onchange={(e) => toggle(role.name, (e.currentTarget as HTMLInputElement).checked)} disabled={applying} />
          <span class="name">{role.name}</span>
          {#if role.dynamic}
            <span class="tag" title="A filter uses USERPRINCIPALNAME(), USERNAME(), USEROBJECTID() or CUSTOMDATA(): check Other user and enter a user name">
              <Icon name="viewAs" size={12} /> per user
            </span>
          {/if}
          {#if role.members != null}
            <span class="muted small">{plural(role.members, 'member')}</span>
          {/if}
        </label>
        {#if role.description}
          <p class="muted small detail">{role.description}</p>
        {/if}
        {#each lines as line, j (j)}
          <p class="filter mono small" title={line}>{line}</p>
        {/each}
      </li>
    {/each}
  </ul>
  <div class="other">
    <label class="check">
      <input type="checkbox" checked={otherUser} onchange={(e) => toggleOther((e.currentTarget as HTMLInputElement).checked)} disabled={applying} />
      <span class="name">Other user</span>
    </label>
    <input
      bind:this={userInput}
      class="user"
      type="text"
      bind:value={userName}
      oninput={() => (error = null)}
      onkeydown={keydown}
      disabled={!otherUser || applying}
      placeholder="someone@contoso.com"
      aria-label="Other user: user principal name"
      spellcheck="false"
      autocomplete="off"
    />
  </div>
  <p class="muted small hint">Other user is what <code>USERPRINCIPALNAME()</code> and <code>USERNAME()</code> return, for roles that filter by user.</p>
  {#if dynamicChecked.length > 0 && !otherUser}
    <p class="note small" role="note">
      <Icon name="info" size={14} />
      {dynamicChecked.map((r) => r.name).join(', ')}
      {dynamicChecked.length === 1 ? 'filters' : 'filter'} by user: without Other user no user matches.
    </p>
  {/if}
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
  {#snippet footer()}
    <button class="primary" onclick={() => void apply()} disabled={applying || needsUser} title={needsUser ? 'Enter the other user' : undefined}>{applying ? 'Applying…' : 'OK'}</button>
    <button onclick={onclose}>Cancel</button>
  {/snippet}
</Modal>

<style>
  .intro {
    margin: 0 0 10px;
    font-size: 12px;
  }

  .roles {
    max-height: 320px;
    margin: 0;
    padding: 4px 0;
    overflow: auto;
    border: 1px solid var(--color-border);
    border-radius: 2px;
    list-style: none;
  }

  .roles li {
    padding: 4px 10px 6px;
  }

  .roles li + li {
    border-top: 1px solid var(--color-border);
  }

  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    cursor: pointer;
  }

  .name {
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .tag {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 0 6px;
    border: 1px solid var(--color-border-strong);
    border-radius: 8px;
    color: var(--color-text-muted);
    font-size: 11px;
    white-space: nowrap;
  }

  .detail,
  .filter {
    margin: 2px 0 0 24px;
  }

  .filter {
    overflow: hidden;
    color: var(--color-text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .other {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 12px;
  }

  .user {
    flex: 1;
    min-width: 0;
    padding: 4px 6px;
    border: 1px solid var(--color-border-strong);
    border-radius: 2px;
    font: inherit;
  }

  .user:disabled {
    background: var(--color-hover);
  }

  .hint {
    margin: 6px 0 0;
  }

  .note {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 10px 0 0;
    padding: 6px 8px;
    border: 1px solid var(--color-warning-border);
    border-radius: 2px;
    background: var(--color-warning-bg);
  }

  .small {
    font-size: 12px;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .error {
    margin: 12px 0 0;
    color: var(--color-danger);
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }
</style>
