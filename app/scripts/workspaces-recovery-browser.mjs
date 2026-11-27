// Mock-only reproduction of a CLI registry change beside a refused workspace
// read. Run against VITE_MOCK=1 vite --port 1497 --strictPort --force.
import assert from 'node:assert/strict';
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright');
const base = process.env.WS_MOCK_URL ?? 'http://127.0.0.1:1497';
for (const [name, engine] of Object.entries({chromium, webkit})) {
 const browser = await engine.launch();
 try {
  const page = await browser.newPage();
  await page.addInitScript(() => sessionStorage.setItem('wt-mock-ui-state', JSON.stringify({settings_rev:4, nav_pinned:true, nav_view:'workspaces'})));
  await page.goto(`${base}/?workspaces=1&activity=0`);
  await page.waitForSelector('[data-testid=ws-group]');
  const before = await page.locator('[data-testid=ws-group]').count();
  await page.evaluate(() => {
   const ipc = window.__TAURI_INTERNALS__, invoke = ipc.invoke;
   window.__workspaceRefused = 0;
   window.__workspaceReads = 0;
   window.__registryPhase = 1;
   window.__refuseWorkspace = false;
   ipc.invoke = async (cmd, args) => {
    if (cmd === 'workspaces_list') {
     window.__workspaceReads++;
     if (window.__refuseWorkspace) { window.__refuseWorkspace = false; window.__workspaceRefused++; throw 'busy'; }
    }
    const reply = await invoke(cmd, args);
    if (cmd === 'list_workspace') {
     reply.projects.push({root:'/scratch/new-unassigned',name:'new-unassigned',ok:true,error:null,snapshot:null});
     if (window.__registryPhase === 2) reply.projects.push({root:'/scratch/broken',name:'broken',ok:false,error:'fixture project',snapshot:null});
    }
    return reply;
   };
   window.__mock.editFile('/nothing'); // emits the poll's places:changed
  });
  await page.waitForFunction(() => window.__workspaceReads > 0);
  assert.equal(await page.locator('[data-testid=ws-group]').count(), before, 'an unassigned project alone keeps headers');
  await page.evaluate(() => {
   window.__registryPhase = 2; window.__refuseWorkspace = true;
   window.__mock.editFile('/nothing');
  });
  await page.waitForFunction(() => window.__workspaceRefused === 1);
  await page.waitForTimeout(150);
  const after = await page.locator('[data-testid=ws-group]').count();
  console.log(`${name}: headers before=${before}, after busy=${after}, switch=${await page.locator('[data-testid=nav-view-workspaces]').count()}`);
  assert.equal(after,before,'a refused refresh must keep the workspace headers');
  await page.waitForFunction(() => window.__workspaceReads >= 3); // automatic retry, no focus needed
  await page.evaluate(() => window.dispatchEvent(new Event('focus')));
  await page.waitForSelector('[data-testid=ws-group]');
  console.log(`ok — ${name}: CLI registry change with one busy read retains Workspaces`);
 } finally { await browser.close(); }
}
