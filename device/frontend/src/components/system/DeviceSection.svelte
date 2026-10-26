<script lang="ts">
  import Segmented from "@/components/ui/Segmented.svelte";
  import { theme, type Theme } from "@/stores/theme.svelte";
  import { snackbar } from "@/stores/snackbar.svelte";
  import { copyText } from "@/lib/clipboard";
  import { message } from "@/api/client";
  import { onMount, onDestroy } from "svelte";
  import { api } from "@/api/client";
  import type { Settings } from "@/types/settings";
  import type { SystemInfo } from "@/types/system";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import TextField from "@/components/ui/TextField.svelte";
  import Select from "@/components/ui/Select.svelte";
  import { btn } from "@/components/ui/buttons";
  import AccessCard from "@/components/system/AccessCard.svelte";
  import SshCard from "@/components/system/SshCard.svelte";
  import PowerCard from "@/components/system/PowerCard.svelte";
  import { developer } from "@/stores/developer.svelte";
  import { settings } from "@/stores/settings.svelte";
  let {
    device = $bindable(),
    tab,
  }: { device: Settings["device"]; tab: string } = $props();

  let info = $state<SystemInfo | null>(null);

  // Against the saved name, not the one being typed
  const renameFailed = $derived(
    !!info?.hostname &&
      !!settings.saved &&
      info.hostname !== settings.saved.device.hostname,
  );
  let timer: ReturnType<typeof setInterval> | null = null;

  async function refresh() {
    try {
      info = await api.system();
    } catch {
      // keep the last values
    }
  }

  function duration(secs: number) {
    const d = Math.floor(secs / 86400);
    const h = Math.floor((secs % 86400) / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return d ? `${d}d ${h}h` : h ? `${h}h ${m}m` : `${m}m`;
  }

  function bytes(n: number) {
    const gb = n / 1024 ** 3;
    return gb >= 1 ? `${gb.toFixed(1)} GB` : `${Math.round(n / 1024 ** 2)} MB`;
  }

  const power = $derived.by(() => {
    const t = info?.throttling;
    if (!t) return null;
    if (t.under_voltage_now) return { text: "Under-voltage now", bad: true };
    if (t.throttled_now) return { text: "Throttled now", bad: true };
    if (t.under_voltage_seen)
      return { text: "Under-voltage since boot", bad: true };
    if (t.throttled_seen) return { text: "Throttled since boot", bad: true };
    return { text: "OK", bad: false };
  });

  const rows = $derived.by(() => {
    if (!info) return [];
    const r: [string, string][] = [];
    if (info.model) r.push(["Board", info.model]);
    if (info.os) r.push(["System", info.os]);
    if (info.kernel) r.push(["Kernel", info.kernel]);
    r.push(["Openstreamer", info.version]);
    if (info.serial) r.push(["Serial", info.serial]);
    if (info.uptime_secs !== null)
      r.push(["Uptime", duration(info.uptime_secs)]);
    if (info.cpu_temp_c !== null)
      r.push(["CPU temperature", `${info.cpu_temp_c.toFixed(1)} °C`]);
    if (info.load)
      r.push([
        "Load (1 / 5 / 15 min)",
        info.load.map((l) => l.toFixed(2)).join(" / "),
      ]);
    if (info.mem_total_kb && info.mem_available_kb !== null) {
      const used = (info.mem_total_kb - info.mem_available_kb) * 1024;
      r.push([
        "Memory used",
        `${bytes(used)} of ${bytes(info.mem_total_kb * 1024)}`,
      ]);
    }
    if (info.disk_total_bytes && info.disk_free_bytes !== null) {
      r.push([
        "Storage free",
        `${bytes(info.disk_free_bytes)} of ${bytes(info.disk_total_bytes)}`,
      ]);
    }
    return r;
  });

  onMount(() => {
    refresh();
    timer = setInterval(refresh, 5000);
  });

  let copyingLogs = $state(false);
  async function copyLogs() {
    copyingLogs = true;
    try {
      await copyText(await api.logs(300));
      snackbar.show("Logs copied.");
    } catch (e) {
      snackbar.show(message(e), true);
    } finally {
      copyingLogs = false;
    }
  }
  onDestroy(() => {
    if (timer) clearInterval(timer);
  });

  // Every name for UTC, so a UTC browser never suggests "changing" to it
  const isUtc = (tz: string) =>
    /^(Etc\/)?(UTC|UCT|GMT([+-]?0)?|Universal|Zulu|Greenwich)$/.test(tz);

  const browserZone = (() => {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
    } catch {
      return "";
    }
  })();

  // Only while the device is still on UTC, and the browser knows better
  const suggestedZone = $derived(
    browserZone &&
      !isUtc(browserZone) &&
      isUtc(device.timezone) &&
      browserZone !== device.timezone
      ? browserZone
      : "",
  );

  const zones = $derived.by(() => {
    let list: string[] = [];
    try {
      list = Intl.supportedValuesOf("timeZone");
    } catch {
      // older browsers: the current zone and the browser's are still offered
    }
    const all = new Set([...list, "Etc/UTC", device.timezone, browserZone]);
    return [...all]
      .filter(Boolean)
      .sort()
      .map((z) => ({ label: z.replaceAll("_", " "), value: z }));
  });

  const THEMES: { label: string; value: Theme }[] = [
    { label: "Auto", value: "auto" },
    { label: "Light", value: "light" },
    { label: "Dark", value: "dark" },
  ];
</script>

<div class="flex flex-col gap-4">
  {#if tab === "general"}
    <Card title="General">
      <Field
        label="Device name"
        for="hostname"
        help="Also the network name: http://{device.hostname ||
          '…'}.local:{location.port || 80}"
      >
        <TextField id="hostname" bind:value={device.hostname} maxlength={63} />
      </Field>
      <Field
        label="Timezone"
        for="timezone"
        help="Used for the date and time in recording names."
      >
        <Select id="timezone" bind:value={device.timezone} options={zones} />
      </Field>
      {#if suggestedZone}
        <div
          class="flex flex-wrap items-center justify-between gap-2 bg-primary/10 px-4 py-2.5 text-[12px] text-fg"
        >
          <span>
            This browser is on <span class="font-mono">{suggestedZone}</span>.
          </span>
          <button
            type="button"
            class={btn.plain}
            onclick={() => (device.timezone = suggestedZone)}
            >Use {suggestedZone.replaceAll("_", " ")}</button
          >
        </div>
      {/if}
      <Field label="Theme" help="Remembered in this browser.">
        <div class="w-full sm:w-64">
          <Segmented
            options={THEMES}
            value={theme.value}
            onchange={(v) => theme.set(v)}
          />
        </div>
      </Field>
      {#if renameFailed}
        <p class="bg-warn/10 px-4 py-3 text-[12px] text-fg">
          The system is still called <span class="font-mono"
            >{info?.hostname}</span
          >, so it answers at
          <span class="font-mono">{info?.hostname}.local</span>. Renaming it
          didn't work; the Logs page says why.
        </p>
      {/if}
    </Card>

    <AccessCard />

    <SshCard />

    <PowerCard />
  {:else if tab === "about"}
    <Card title="About">
      {#if !info}
        <p class="px-4 py-3 text-[13px] text-muted">Loading…</p>
      {:else}
        <dl>
          {#each rows as [k, v]}
            {@render row(
              k,
              v,
              false,
              k === "Openstreamer" ? () => developer.tap() : undefined,
            )}
          {/each}
          {#if power}
            {@render row("Power", power.text, power.bad)}
          {/if}
          <div
            class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 border-t border-line px-4 py-2.5"
          >
            <dt class="text-[13px] text-fg">Diagnostics</dt>
            <dd
              class="flex flex-wrap items-baseline justify-end gap-x-3 text-[13px]"
            >
              <button
                type="button"
                disabled={copyingLogs}
                onclick={copyLogs}
                class="text-primary hover:underline disabled:opacity-50"
                >{copyingLogs ? "Copying…" : "Copy logs"}</button
              >
            </dd>
          </div>
        </dl>
      {/if}
    </Card>
  {/if}
</div>

{#snippet row(k: string, v: string, bad = false, onclick?: () => void)}
  <div
    class="relative flex items-baseline justify-between gap-4 border-t border-line px-4 py-2.5 first:border-t-0"
  >
    <dt class="text-[13px] text-fg">{k}</dt>
    <dd
      class="truncate text-right text-[13px] tabular-nums {bad
        ? 'text-down'
        : 'text-muted'}"
    >
      {#if onclick}
        <button
          type="button"
          {onclick}
          class="select-none after:absolute after:inset-0 focus-visible:outline-none focus-visible:after:outline-2 focus-visible:after:-outline-offset-2 focus-visible:after:outline-primary"
          >{v}</button
        >
      {:else}
        {v}
      {/if}
    </dd>
  </div>
{/snippet}
