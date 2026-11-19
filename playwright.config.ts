import {defineConfig} from '@playwright/test';
export default defineConfig({
  testDir: './tests',
  workers: 1,
  timeout: 150000,
  use: {
    baseURL: 'http://127.0.0.1:1420',
    viewport: {width: 980, height: 730},
    launchOptions: {
      executablePath:
        '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    },
  },
  webServer: {
    command: 'npm run dev',
    url: 'http://127.0.0.1:1420',
    reuseExistingServer: false,
  },
});
