import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';

const here = (path: string) => fileURLToPath(new URL(path, import.meta.url));

// Two windows, two entry points. Tauri opens them by file name
// (`WebviewUrl::App("index.html")` and `"dashboard.html"`), so both have to land
// at the root of the output directory under exactly those names — which is what
// Vite does with multiple HTML inputs.
//
// The toast is deliberately not a React page. It opens on the critical path of
// an agent waiting for an answer, and the tray app pre-warms a hidden copy at
// startup to hide WebView2's cold start; a framework there would buy nothing and
// cost first paint. It is still built here so there is one pipeline, not two.
export default defineConfig({
    root: 'ui',
    // Tauri serves the built files from disk, so asset URLs have to be relative
    // rather than rooted at "/".
    base: './',
    plugins: [react(), tailwindcss()],
    build: {
        outDir: here('./dist'),
        emptyOutDir: true,
        // WebView2 on Windows 10+ is evergreen Chromium; nothing needs to reach
        // further back than that.
        target: 'chrome120',
        rollupOptions: {
            input: {
                toast: here('./ui/index.html'),
                dashboard: here('./ui/dashboard.html'),
            },
        },
    },
    server: {
        port: 1420,
        // A moved port is a `tauri dev` window pointed at nothing, which is
        // worse than a clear failure to start.
        strictPort: true,
    },
});
