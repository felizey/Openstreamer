import type { APIRoute } from "astro";

export const prerender = false;

export const GET: APIRoute = () =>
  new Response("Install the local test package with sudo apt install ./openstreamer_*.deb. This fork has no public installation server.", {
    status: 503,
    headers: { "Cache-Control": "no-store", "Content-Type": "text/plain; charset=utf-8" },
  });
