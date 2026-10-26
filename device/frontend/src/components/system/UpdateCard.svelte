<script lang="ts">
  import { onMount } from "svelte";
  import { api, message } from "@/api/client";
  import type { UpdateStatus } from "@/types/system";
  import { snackbar } from "@/stores/snackbar.svelte";
  import {
    compareVersions,
    OPENSTREAMER_LATEST_RELEASE_API,
    parseLatestRelease,
    type GitHubRelease,
  } from "@/lib/githubRelease";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import ConfirmDialog from "@/components/ui/ConfirmDialog.svelte";
  import { btn } from "@/components/ui/buttons";

  const POLL_MS = 2000;

  let status = $state<UpdateStatus | null>(null);
  let release = $state<GitHubRelease | null>(null);
  let releaseChecking = $state(false);
  let releaseError = $state<string | null>(null);
  let releaseComparison = $derived(
    status && release ? compareVersions(release.tag, status.current) : null,
  );
  let busy = $state(false);
  /** The version being installed, from the request until it's running */
  let installing = $state<string | null>(null);
  /** Seen underway or restarting, so the same version reported again means done */
  let underway = false;
  let confirm = $state<"install" | "rollback" | null>(null);
  let confirmOpen = $state(false);
  let input: HTMLInputElement;

  onMount(() => {
    refresh();
    const timer = setInterval(() => installing && refresh(), POLL_MS);
    return () => clearInterval(timer);
  });

  async function checkRelease() {
    releaseChecking = true;
    releaseError = null;
    release = null;
    try {
      const response = await fetch(OPENSTREAMER_LATEST_RELEASE_API, {
        headers: {
          Accept: "application/vnd.github+json",
          "X-GitHub-Api-Version": "2022-11-28",
        },
        cache: "no-store",
        signal: AbortSignal.timeout(12000),
      });
      if (response.status === 404) {
        throw new Error(
          "No stable Openstreamer release has been published yet.",
        );
      }
      if (response.status === 403) {
        throw new Error(
          "GitHub is temporarily limiting release checks. Try again later.",
        );
      }
      if (!response.ok) {
        throw new Error(`GitHub release check failed (${response.status}).`);
      }
      const latest = parseLatestRelease(await response.json());
      if (!latest)
        throw new Error("GitHub returned an invalid release record.");
      release = latest;
    } catch (error) {
      releaseError =
        error instanceof TypeError ||
        (error instanceof DOMException && error.name === "TimeoutError")
          ? "Can't reach GitHub. Check the internet connection and try again."
          : error instanceof Error
            ? error.message
            : "Couldn't check for an Openstreamer release.";
    } finally {
      releaseChecking = false;
    }
  }

  async function refresh() {
    let next: UpdateStatus;
    try {
      next = await api.update();
    } catch {
      // Expected while the service restarts into the new version
      underway = true;
      return;
    }
    status = next;
    if (!installing) return;
    if (next.installing) {
      underway = true;
    } else if (underway && next.current === installing && !next.failure) {
      // This page is the old version's; the new one comes with a reload
      location.reload();
    } else if (next.failure) {
      installing = null;
    }
  }

  async function upload(e: Event) {
    const files = [...((e.currentTarget as HTMLInputElement).files ?? [])];
    input.value = "";
    if (!files.length) return;
    const bundle = files.find((f) => f.name.toLowerCase().endsWith(".osupd"));
    const deb = files.find((f) => f.name.toLowerCase().endsWith(".deb"));
    const sig = files.find((f) => f.name.toLowerCase().endsWith(".sig"));
    if (!(
      (files.length === 1 && bundle) ||
      (files.length === 2 && deb && sig)
    )) {
      snackbar.show(
        "Choose one .osupd file, or both the .deb and .sig files",
        true,
      );
      return;
    }
    busy = true;
    try {
      status = bundle
        ? await api.uploadUpdate(bundle)
        : await api.uploadUpdate(deb!, (await sig!.text()).trim());
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      busy = false;
    }
  }

  async function discard() {
    try {
      status = await api.discardUpdate();
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }

  function ask(what: "install" | "rollback") {
    confirm = what;
    confirmOpen = true;
  }

  async function run() {
    const version = confirm === "install" ? status?.staged : status?.previous;
    if (!version) return;
    try {
      await (confirm === "install"
        ? api.installUpdate()
        : api.rollBackUpdate());
      underway = false;
      installing = version;
    } catch (e) {
      snackbar.show(message(e), true);
    }
  }
</script>

<Card title="Updates">
  {#if !status}
    <p class="px-4 py-3 text-[13px] text-muted">Loading…</p>
  {:else if installing}
    <p class="px-4 py-3 text-[13px] text-muted">
      Installing {installing}… Video stops while Openstreamer restarts, and this
      page reloads when it's done.
    </p>
  {:else}
    <Field label="Installed version">
      <span class="text-[13px] text-muted tabular-nums">{status.current}</span>
    </Field>
    <Field
      label="Openstreamer releases"
      help="Checks the latest stable release from felizey/Openstreamer. Updates are still verified before installation."
    >
      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          class={btn.plain}
          disabled={releaseChecking}
          onclick={checkRelease}
        >
          {releaseChecking ? "Checking…" : "Check now"}
        </button>
        {#if release}
          <a
            class={btn.text}
            href={release.url}
            aria-label={`Open release notes for ${release.title}`}
            target="_blank"
            rel="noreferrer"
          >
            Release notes
          </a>
        {/if}
      </div>
    </Field>
    {#if releaseError}
      <p class="px-4 py-3 text-[13px] text-muted">{releaseError}</p>
    {:else if release}
      <div class="space-y-2 px-4 py-3">
        <p class="text-[13px] text-fg">
          {#if releaseComparison === null}
            Latest stable release: {release.tag}. This build's version could not
            be compared.
          {:else if releaseComparison > 0}
            Version {release.tag} is available.
          {:else if releaseComparison === 0}
            You are running the latest stable release ({release.tag}).
          {:else}
            No newer stable release is available (latest: {release.tag}).
          {/if}
        </p>
        {#if release.assets.length}
          <div class="flex flex-wrap gap-2">
            {#each release.assets as asset (asset.name)}
              <a class={btn.plain} href={asset.url} download={asset.name}>
                Download {asset.name}
              </a>
            {/each}
          </div>
          <p class="text-[12px] text-muted">
            Choose the downloaded signed .osupd below. If the release only has
            .deb and .sig files, choose both together.
          </p>
        {:else}
          <p class="text-[12px] text-muted">
            This release has no signed update files attached yet.
          </p>
        {/if}
      </div>
    {/if}
    {#if status.staged}
      <Field
        label="Ready to install"
        help={status.staged === status.current
          ? "This is the version already installed."
          : undefined}
      >
        <div class="flex items-center gap-2">
          <span class="text-[13px] text-fg tabular-nums">{status.staged}</span>
          <button type="button" class={btn.text} onclick={discard}>
            Discard
          </button>
          <button
            type="button"
            class={btn.primary}
            onclick={() => ask("install")}
          >
            Install
          </button>
        </div>
      </Field>
    {:else}
      <Field
        label="Install from file"
        help="Choose one signed .osupd update. A .deb package with its .sig is also accepted. Settings and recordings are kept."
      >
        <button
          type="button"
          class={btn.plain}
          disabled={busy}
          onclick={() => input.click()}
        >
          {busy ? "Verifying…" : "Choose update…"}
        </button>
      </Field>
    {/if}
    {#if status.previous}
      <Field label="Previous version">
        <div class="flex items-center gap-2">
          <span class="text-[13px] text-muted tabular-nums">
            {status.previous}
          </span>
          <button
            type="button"
            class={btn.plain}
            onclick={() => ask("rollback")}
          >
            Roll back
          </button>
        </div>
      </Field>
    {/if}
    {#if status.failure}
      <div class="border-t border-line px-4 py-3">
        <p class="text-[13px] text-down">
          {#if status.failure.reverted}
            Version {status.failure.version} didn't start, so the device went back
            to {status.current}.
          {:else}
            Installing {status.failure.version} failed.
          {/if}
        </p>
        {#if status.failure.log}
          <pre
            class="mt-2 max-h-48 overflow-auto rounded-lg bg-surface-2 p-3 font-mono text-[11px] whitespace-pre-wrap text-muted">{status
              .failure.log}</pre>
        {/if}
      </div>
    {/if}
  {/if}
  <input
    bind:this={input}
    type="file"
    accept=".osupd,.deb,.sig"
    multiple
    class="hidden"
    onchange={upload}
  />
</Card>

<ConfirmDialog
  bind:open={confirmOpen}
  title={confirm === "install"
    ? `Install ${status?.staged}?`
    : `Roll back to ${status?.previous}?`}
  message="Video, streams and any recording stop while Openstreamer restarts. Don't unplug the device until it's back."
  confirmLabel={confirm === "install" ? "Install" : "Roll back"}
  onconfirm={run}
/>
