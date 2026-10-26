<script lang="ts">
  import { onMount } from "svelte";
  import { api, message } from "@/api/client";
  import type { Settings } from "@/types/settings";
  import { snackbar } from "@/stores/snackbar.svelte";
  import Card from "@/components/ui/Card.svelte";
  import Field from "@/components/ui/Field.svelte";
  import Switch from "@/components/ui/Switch.svelte";
  import Segmented from "@/components/ui/Segmented.svelte";
  import { btn } from "@/components/ui/buttons";
  let { splash = $bindable() }: { splash: Settings["splash"] } = $props();

  const SCALING = [
    { label: "Fit", value: "fit" },
    { label: "Fill", value: "fill" },
  ];
  const WIDTH = 1920;
  const HEIGHT = 1080;

  const MAX_VIDEO_BYTES = 256 * 1024 * 1024;

  let customImage = $state(false);
  let customVideo = $state(false);
  let busy = $state(false);
  let phase = $state<"preparing" | "uploading" | "processing" | null>(null);
  let progress = $state(0);
  let uploadingVideo = $state(false);
  let error = $state<string | null>(null);
  let version = $state(Date.now());
  let previewFailed = $state(false);
  let imageInput: HTMLInputElement;
  let videoInput: HTMLInputElement;

  const custom = $derived(customImage || customVideo);
  const working = $derived(
    phase === "preparing"
      ? "Preparing image…"
      : phase === "uploading"
        ? `Uploading… ${progress}%`
        : uploadingVideo
          ? "Converting video on the device… This can take several minutes."
          : "Saving image…",
  );

  onMount(async () => {
    try {
      const status = await api.splash();
      customImage = status.custom_image;
      customVideo = status.custom_video;
    } catch {
      // the switches still work without it
    }
  });

  /** No bigger than it needs to be to cover the screen, in its own shape:
   *  fitting or filling happens on the device, so it can be changed later */
  async function sizedForScreen(file: File): Promise<Blob> {
    const url = URL.createObjectURL(file);
    try {
      const img = new Image();
      img.src = url;
      await img.decode();
      const scale = Math.min(
        1,
        Math.max(WIDTH / img.naturalWidth, HEIGHT / img.naturalHeight),
      );
      const canvas = document.createElement("canvas");
      canvas.width = Math.round(img.naturalWidth * scale);
      canvas.height = Math.round(img.naturalHeight * scale);
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("Couldn't process the image");
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      ctx.imageSmoothingQuality = "high";
      ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
      return await new Promise((resolve, reject) =>
        canvas.toBlob(
          (b) =>
            b ? resolve(b) : reject(new Error("Couldn't process the image")),
          "image/jpeg",
          0.92,
        ),
      );
    } catch (e) {
      throw e instanceof Error && e.message.startsWith("Couldn't")
        ? e
        : new Error("That file isn't an image this browser can open");
    } finally {
      URL.revokeObjectURL(url);
    }
  }

  async function upload(e: Event, video: boolean) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    (e.currentTarget as HTMLInputElement).value = "";
    if (!file) return;
    error = null;
    if (video && file.size > MAX_VIDEO_BYTES) {
      error = "Choose a video up to 256 MiB.";
      return;
    }
    busy = true;
    uploadingVideo = video;
    progress = 0;
    phase = video ? "uploading" : "preparing";
    try {
      const body = video ? file : await sizedForScreen(file);
      phase = "uploading";
      const report = (percent: number) => {
        progress = percent;
        phase = percent >= 100 ? "processing" : "uploading";
      };
      await (video
        ? api.uploadSplashVideo(body, report)
        : api.uploadSplash(body, report));
      customImage = !video;
      customVideo = video;
      refreshPreview();
      snackbar.show(
        video ? "Splash video converted and saved" : "Splash image updated",
      );
    } catch (e) {
      error = message(e);
    } finally {
      busy = false;
      phase = null;
    }
  }

  async function remove() {
    busy = true;
    error = null;
    try {
      await (customVideo ? api.removeSplashVideo() : api.removeSplash());
      customImage = false;
      customVideo = false;
      refreshPreview();
      snackbar.show("Back to the default splash screen");
    } catch (e) {
      error = message(e);
    } finally {
      busy = false;
    }
  }

  function refreshPreview() {
    previewFailed = false;
    version = Date.now();
  }
</script>

<Card title="Splash screen">
  <div class="flex flex-col gap-3 px-4 py-3">
    <div
      class="relative aspect-video w-full overflow-hidden rounded-lg border border-line bg-black sm:max-w-md"
    >
      {#if previewFailed}
        <p
          class="absolute inset-0 flex items-center justify-center text-[12px] text-muted"
        >
          Preview not available yet
        </p>
      {:else}
        <img
          src={api.splashPreviewUrl(
            version,
            splash.status_over_image,
            splash.image_scaling,
          )}
          alt="Splash screen preview"
          class="h-full w-full {customImage && splash.image_scaling === 'fill'
            ? 'object-cover'
            : 'object-contain'}"
          onerror={() => (previewFailed = true)}
        />
      {/if}
    </div>
    {#if customVideo}
      <p class="text-[12px] leading-snug text-muted">
        Video splash screen. Preview shows its first frame; the video loops
        while waiting for a source.
      </p>
    {/if}
    <div class="flex flex-wrap items-center gap-2">
      <button
        type="button"
        class={btn.plain}
        disabled={busy}
        onclick={() => imageInput.click()}
      >
        {customImage ? "Replace image" : "Upload image"}
      </button>
      <button
        type="button"
        class={btn.plain}
        disabled={busy}
        onclick={() => videoInput.click()}
      >
        {customVideo ? "Replace video" : "Upload video"}
      </button>
      {#if custom}
        <button type="button" class={btn.text} disabled={busy} onclick={remove}>
          Use default
        </button>
      {/if}
    </div>
    <p class="text-[12px] leading-snug text-muted">
      Video: up to 256 MiB and 5 minutes. Formats supported by the device are
      converted to MP4, H.264 High Profile, 1920×1080, 30 FPS, about 4 Mbit/s,
      without B-frames and with a keyframe every second. A source MP4 must
      contain H.264 video. Uploading an image or video replaces the current
      custom splash.
    </p>
    {#if busy && phase}
      <div class="flex flex-col gap-2" role="status" aria-live="polite">
        <p class="text-[12px] text-muted">{working}</p>
        {#if phase === "uploading"}
          <progress
            class="h-2 w-full max-w-md accent-primary"
            aria-label="Splash upload progress"
            value={progress}
            max="100"
          ></progress>
        {:else}
          <progress
            class="h-2 w-full max-w-md accent-primary"
            aria-label={phase === "preparing"
              ? "Preparing image"
              : "Processing splash"}
          ></progress>
        {/if}
      </div>
    {/if}
    {#if error}
      <p class="text-[13px] text-down" role="alert">{error}</p>
    {/if}
    <input
      bind:this={imageInput}
      type="file"
      accept="image/*"
      class="hidden"
      onchange={(e) => upload(e, false)}
    />
    <input
      bind:this={videoInput}
      type="file"
      class="hidden"
      onchange={(e) => upload(e, true)}
    />
  </div>

  {#if customImage}
    <Field label="Image scaling">
      <div class="w-full sm:w-64">
        <Segmented
          options={SCALING}
          value={splash.image_scaling}
          onchange={(v) => (splash.image_scaling = v)}
        />
      </div>
    </Field>
  {/if}
  <Field
    label="Show status over custom image"
    for="splash-status"
    help="Keeps addresses and connection status visible on a custom image. This option does not apply to video."
    disabled={!customImage}
  >
    <Switch
      id="splash-status"
      bind:checked={splash.status_over_image}
      disabled={!customImage}
    />
  </Field>
  <Field label="Show access point password on screen" for="splash-ap-password">
    <Switch id="splash-ap-password" bind:checked={splash.show_ap_password} />
  </Field>
  <Field label="Freeze on last frame" for="splash-freeze">
    <Switch id="splash-freeze" bind:checked={splash.freeze_last_frame} />
  </Field>
  <Field
    label="Grayscale frozen frame"
    for="splash-gray"
    help="Like the Fly app, so a frozen picture can't be mistaken for live video."
    disabled={!splash.freeze_last_frame}
  >
    <Switch
      id="splash-gray"
      bind:checked={splash.grayscale_freeze}
      disabled={!splash.freeze_last_frame}
    />
  </Field>
</Card>
