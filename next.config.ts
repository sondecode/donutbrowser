import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  reactStrictMode: true,
  output: "export",
  images: {
    unoptimized: true,
  },
  // Tauri embeds everything under `dist/` as static assets at compile time
  // (`frontendDist: "../dist"`), so the dev server's scratch state must not live
  // there: `next dev` creates and deletes `dist/dev/lock` as it starts and stops,
  // and `generate_context!()` fails when it enumerates that file and then can't
  // read it. Keeping dev output in `.next/` also stops ~300MB of dev cache from
  // being embedded in the binary.
  distDir: process.env.NODE_ENV === "development" ? ".next" : "dist",
  compiler: {
    removeConsole: process.env.NODE_ENV === "production",
  },
};

export default nextConfig;
