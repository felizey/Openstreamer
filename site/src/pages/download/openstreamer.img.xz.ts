import type { APIRoute } from "astro";

export const prerender = false;

export const GET: APIRoute = () =>
  new Response(null, {
    status: 302,
    headers: { Location: "/en/docs/getting-started/install-image", "Cache-Control": "no-store" },
  });
