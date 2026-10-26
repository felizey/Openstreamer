<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { metrics } from "@/stores/metrics.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Chart, { type ChartSeries } from "@/components/ui/Chart.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";

  const RANGES = [
    { label: "1 min", value: 60 },
    { label: "5 min", value: 300 },
    { label: "15 min", value: 900 },
    { label: "30 min", value: 1800 },
  ];

  const PALETTE = [
    "var(--color-chart-1)",
    "var(--color-chart-2)",
    "var(--color-chart-3)",
    "var(--color-chart-4)",
    "var(--color-chart-5)",
    "var(--color-chart-6)",
  ];
  const [SOURCE, APP, SECONDARY, EXTRA, CLOCK, ALERT] = PALETTE;

  let windowSecs = $state(readRange());

  function readRange() {
    try {
      const v = Number(localStorage.getItem("openstreamer.perf.range"));
      return RANGES.some((r) => r.value === v) ? v : 300;
    } catch {
      return 300;
    }
  }

  function setRange(v: number) {
    windowSecs = v;
    try {
      localStorage.setItem("openstreamer.perf.range", String(v));
    } catch {}
  }

  onMount(() => metrics.start());
  onDestroy(() => metrics.stop());

  const s = $derived(metrics.series);

  /** The named readings that exist, as chart lines */
  function lines(
    wanted: [name: string, label: string, color: string, fill?: boolean][],
  ): ChartSeries[] {
    return wanted
      .filter(([name]) => s[name])
      .map(([name, label, color, fill]) => ({
        label,
        color,
        values: s[name],
        fill,
      }));
  }

  const cpu = $derived(
    lines([
      ["cpu", "Total", SOURCE, true],
      ["cpu_app", "Openstreamer", APP],
      ["gpu", "GPU (3D)", EXTRA],
    ]),
  );
  const cores = $derived(
    Object.keys(s)
      .filter((name) => /^cpu\d+$/.test(name))
      .sort()
      .map((name, i) => ({
        label: `Core ${name.slice(3)}`,
        color: PALETTE[i % PALETTE.length],
        values: s[name],
      })),
  );
  const memory = $derived(
    lines([
      ["mem_used", "In use", SOURCE, true],
      ["mem_app", "Openstreamer", APP],
    ]),
  );
  const memTotal = $derived(
    s.mem_total?.findLast((v) => v !== null) ?? undefined,
  );
  const temperature = $derived(lines([["temp", "Processor", ALERT, true]]));
  const clock = $derived(lines([["freq", "Clock", CLOCK, true]]));
  const frameRate = $derived(
    lines([
      ["fps_in", "Source", SOURCE, true],
      ["fps_hdmi", "HDMI", SECONDARY],
      ["hdmi_dropped", "HDMI dropped", ALERT],
    ]),
  );
  const bitrate = $derived(
    lines([
      ["kbps_in", "Source", SOURCE, true],
      ["kbps_rtmp", "RTMP", APP],
    ]),
  );
  const viewers = $derived(
    lines([
      ["viewers_web", "Browsers", SOURCE],
      ["viewers_rtsp", "RTSP", SECONDARY],
      ["viewers_srt", "SRT", APP],
      ["udp_destinations", "UDP destinations", EXTRA],
      ["recording", "Recording", ALERT],
    ]),
  );
  const wifi = $derived(lines([["wifi_dbm", "Signal", SECONDARY]]));

  const interfaces = $derived(
    [
      ...new Set(
        Object.keys(s)
          .filter((name) => name.startsWith("net."))
          .map((name) => name.split(".")[1]),
      ),
    ].sort(),
  );
  const traffic = (direction: "tx" | "rx") =>
    interfaces.map((name, i) => ({
      label: name,
      color: PALETTE[i % PALETTE.length],
      values: s[`net.${name}.${direction}`] ?? [],
      fill: interfaces.length === 1,
    }));
  const sent = $derived(traffic("tx"));
  const received = $derived(traffic("rx"));

  const underVoltage = $derived.by(() => {
    const flags = s.undervoltage;
    if (!flags) return false;
    const since = metrics.t[metrics.t.length - 1] - windowSecs * 1000;
    return flags.some((v, i) => v === 1 && metrics.t[i] >= since);
  });

  const percent = (v: number) => `${Math.round(v)}%`;
  const megabytes = (v: number) =>
    v >= 1024 ? `${(v / 1024).toFixed(1)} GB` : `${Math.round(v)} MB`;
  const degrees = (v: number) => `${v.toFixed(1)} °C`;
  const megahertz = (v: number) => `${Math.round(v)} MHz`;
  const fps = (v: number) => `${Math.round(v)} fps`;
  const rate = (kbps: number) =>
    kbps >= 1000
      ? `${(kbps / 1000).toFixed(1)} Mbit/s`
      : `${Math.round(kbps)} kbit/s`;
  const count = (v: number) => String(Math.round(v));
  const dbm = (v: number) => `${Math.round(v)} dBm`;
</script>

<div class="flex flex-col gap-4">
  <div class="flex flex-wrap items-center justify-between gap-3">
    <p class="text-[12px] text-muted">
      Recorded every second since the service started; the last half hour is
      kept.
    </p>
    <div class="w-full sm:w-80">
      <Segmented options={RANGES} value={windowSecs} onchange={setRange} />
    </div>
  </div>

  {#if !metrics.loaded}
    <p class="text-[13px] text-muted">Loading…</p>
  {:else}
    {#if underVoltage}
      <p class="rounded-lg bg-down/10 px-4 py-3 text-[13px] text-down">
        The power supply dipped below what the Pi needs during this period. That
        causes glitches and USB drop-outs; use a stronger supply or a shorter
        cable.
      </p>
    {/if}

    <div class="grid gap-4 lg:grid-cols-2">
      {#snippet chart(
        title: string,
        series: ChartSeries[],
        format: (v: number) => string,
        max?: number,
        min?: number,
      )}
        {#if series.length}
          <Card {title}>
            <Chart t={metrics.t} {series} {windowSecs} {format} {max} {min} />
          </Card>
        {/if}
      {/snippet}

      {@render chart("Processor", cpu, percent, 100)}
      {@render chart("Processor cores", cores, percent, 100)}
      {@render chart("Memory", memory, megabytes, memTotal)}
      {@render chart("Temperature", temperature, degrees, 90, 30)}
      {@render chart("Frame rate", frameRate, fps)}
      {@render chart("Video bitrate", bitrate, rate)}
      {@render chart("Network sent", sent, rate)}
      {@render chart("Network received", received, rate)}
      {@render chart("Viewers and outputs", viewers, count)}
      {@render chart("Processor clock", clock, megahertz)}
      {@render chart("WiFi signal", wifi, dbm, -30, -90)}
    </div>
  {/if}
</div>
