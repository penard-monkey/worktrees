// Optional browser gate. Install Playwright in scratch space and its WebKit /
// Chromium engines, then run against pnpm dev:mock --port 1438 --force:
// PLAYWRIGHT_MODULE=/tmp/browser/node_modules/playwright/index.mjs node app/scripts/plan-usage-browser.mjs
// USAGE_SCREENSHOTS optionally saves strip screenshots at three viewport widths.
// USAGE_MOCK_URL overrides the URL; USAGE_REPORT optionally saves full measurements.
const { chromium, webkit } = await import(process.env.PLAYWRIGHT_MODULE ?? "playwright");
const baseURL = process.env.USAGE_MOCK_URL ?? "http://localhost:1438";
import assert from 'node:assert/strict';
import fs from 'node:fs';
const results = [];
for (const [engine, type] of Object.entries({chromium, webkit})) {
 const browser = await type.launch({headless:true});
 const context = await browser.newContext({viewport:{width:1280,height:800}});
 const page=await context.newPage();
 const errors=[]; page.on('pageerror', e=>errors.push(String(e)));
 await page.goto(baseURL+'/?codexUsage=multi');
 await page.bringToFront();
 await page.waitForSelector('[data-testid="usage-meter"]');
 await page.waitForFunction(()=>document.querySelector('[data-testid="usage-meter"]').textContent.includes('48%'));
 const meter=page.locator('[data-testid="usage-meter"]');
 await meter.click();
 await page.waitForSelector('.usage-pop.pinned');
 const layout=await page.evaluate(()=>{
  const button=document.querySelector('.usage-trig'), inner=button.querySelector('.usage-shape');
  return { button:button.getBoundingClientRect().width, inner:inner.getBoundingClientRect().width,
   labels:[...button.querySelectorAll('.usage-provider-name,.usage-window-label')].map(e=>({text:e.textContent,width:e.getBoundingClientRect().width})),
   bars:[...button.querySelectorAll('.usage-bar')].map(e=>e.getBoundingClientRect().width),
   reached:[...document.querySelectorAll('.usage-provider-detail')].every(e=>{const r=e.getBoundingClientRect();return e.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height-2));})};
 });
 assert.deepEqual(layout.labels.filter(l => !['Claude','Codex'].includes(l.text)).map(l => l.text),
  ['5h','7d','Fable 7d','5h','7d','gpt-reserve with a very long bucket label 7d'],
  'strip shows every provider window in stable order');
 assert(layout.labels.every(l=>l.width>0)); assert(layout.bars.every(w=>w===40)); assert(layout.button>=layout.inner); assert(layout.reached);
 results.push({engine,layout});
 for (const theme of ['tokyo-night','tokyo-day','catppuccin-mocha','catppuccin-latte','nord','gruvbox-dark']) {
  await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
  const contrast=await page.evaluate(()=>{
   function rgba(s) {
    if(s.startsWith('color(srgb')) { const v=s.slice(11,-1).trim().split(/[\s/]+/).map(Number);return [v[0]*255,v[1]*255,v[2]*255,v[3]??1]; }
    const v=s.match(/[\d.]+/g)?.map(Number)??[0,0,0,0];return [v[0],v[1],v[2],v[3]??1];
   }
   const over=(a,b)=>[0,1,2].map(i=>a[i]*a[3]+b[i]*(1-a[3])).concat(1);
   function background(e) { const layers=[];for(let n=e;n;n=n.parentElement)layers.push(rgba(getComputedStyle(n).backgroundColor));return layers.reverse().reduce((b,a)=>over(a,b),[255,255,255,1]); }
   const lum=c=>c.slice(0,3).map(v=>v/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4).reduce((s,v,i)=>s+v*[.2126,.7152,.0722][i],0);
   const swatch=document.createElement('span');document.body.append(swatch);
   const neutral=['--txt-hi','--txt-dim'].map(token=>{swatch.style.color=`var(${token})`;return getComputedStyle(swatch).color;});swatch.remove();
   const text = [...document.querySelectorAll('.usage-pop .usage-label,.usage-pop .usage-pct,.usage-pop .usage-eta,.usage-provider-head,.usage-pop-head,.usage-provider-name')].map(e=>{
    const cs=getComputedStyle(e),bg=background(e),fg=over(rgba(cs.color),bg);const a=lum(fg),b=lum(bg);
    return {text:e.textContent,ratio:(Math.max(a,b)+.05)/(Math.min(a,b)+.05),color:cs.color,neutral:neutral.includes(cs.color)};
   });
   const divider=document.querySelector('.usage-provider-summary + .usage-provider-summary');
   const cs=getComputedStyle(divider), bg=background(divider), fg=over(rgba(cs.borderLeftColor),bg);
   const a=lum(fg), b=lum(bg);
   return {text,divider:{ratio:(Math.max(a,b)+.05)/(Math.min(a,b)+.05),width:parseFloat(cs.borderLeftWidth),height:divider.getBoundingClientRect().height}};
  });
  assert(contrast.divider.ratio>=3 && contrast.divider.width===2 && contrast.divider.height>=18,
   `${engine} ${theme}: provider divider must be visible`);
  assert(contrast.text.every(c=>c.neutral), "Usage text must use neutral tokens, never severity accents");
  results.push({engine,theme,minContrast:Math.min(...contrast.text.map(c=>c.ratio)),divider:contrast.divider,contrast});
 }
 await page.keyboard.press('Escape'); assert.equal(await page.locator('.usage-pop.pinned').count(),0);
 for (const theme of ['tokyo-night','tokyo-day']) {
  await page.evaluate(theme=>document.documentElement.dataset.theme=theme,theme);
  for (const width of [1280,900,600]) {
   await page.setViewportSize({width,height:800});
   const compact=await meter.evaluate(e=>{
    const visible=n=>n.getBoundingClientRect().width>0 && n.getBoundingClientRect().height>0;
    const r=e.getBoundingClientRect();
    return {width:r.width,right:r.right,windows:[...e.querySelectorAll('.usage-window')].filter(visible).map(n=>n.textContent),
     bars:[...e.querySelectorAll('.usage-bar')].filter(visible).map(n=>n.getBoundingClientRect().width),
     reachable:[...e.querySelectorAll('.usage-window')].filter(visible).every(n=>{const r=n.getBoundingClientRect();return e.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));})};
   });
   assert(compact.right<=width && compact.reachable);
   assert.equal(compact.windows.length,width===1280?6:2);
   assert.equal(compact.bars.length,width===1280?6:width===900?2:0);
   if(width<1280) assert.deepEqual(compact.windows,['Fable 7d80%','5h48%']);
   results.push({engine,theme,viewport:width,compact});
   if(process.env.USAGE_SCREENSHOTS && engine==='webkit') {
    fs.mkdirSync(process.env.USAGE_SCREENSHOTS,{recursive:true});
    await page.locator('.usage-strip').screenshot({path:`${process.env.USAGE_SCREENSHOTS}/${theme}-${width}.png`});
   }
  }
 }
 await page.setViewportSize({width:1280,height:800});

 for (const mode of ['weekly','stale','expired','old','unavailable','signedout','missing','unsupported','edge']) {
  await page.goto(`${baseURL}/?codexUsage=${mode}`); await page.bringToFront();
  await page.waitForFunction(()=>window.__planUsageCalls?.codex_usage>0);
  await page.locator('[data-testid="usage-meter"]').click();
  const detail=await page.locator('[data-provider="codex"]').innerText();
  if(mode==='expired')assert(detail.includes('Awaiting update'));
  if(mode==='old'){assert(detail.includes('Could not read usage'));assert(!detail.includes('48%'));}
  if(mode==='missing'){
   assert(!await meter.innerText().then(t=>t.includes('Codex')));
   assert.equal(detail,'Codex CLI was not found.');
   assert.equal(await page.locator('.usage-provider-detail[data-provider="codex"]').count(),0);
  }
  if(mode==='signedout')assert(detail.includes('Not signed in'));
  if(mode==='weekly'){assert(detail.includes('7d'));assert(!detail.includes('5h'));}
  if(mode==='edge')assert(detail.includes('105%'));
  results.push({engine,mode,detail});
 }
 // With only Codex enabled, keep a generic details trigger for the missing-CLI note.
 await page.evaluate(()=>sessionStorage.setItem('wt-mock-ui-state',JSON.stringify({usage_claude:false,usage_codex:true})));
 await page.goto(baseURL+'/?codexUsage=missing'); await page.bringToFront();
 await page.waitForFunction(()=>window.__planUsageCalls?.codex_usage>0);
 await page.waitForFunction(()=>document.querySelector('.usage-minimal.only'));
 assert.equal((await meter.innerText()).trim(),'Usage ▾');
 await meter.click();
 assert.equal(await page.locator('[data-provider="codex"]').innerText(),'Codex CLI was not found.');
 results.push({engine,missingCliOnly:'generic trigger and one-line note'});
 await page.goto(baseURL+'/?codexUsage=ready');
 // `goto` resolves at `load`, which does NOT wait for main.tsx's `import("./App")`.
 // Reloading before App renders cancels that import mid-fetch, and WebKit reports
 // the abandoned import as an unhandled rejection (Chromium drops it silently) — an
 // intermittent failure in the gate, not the app. Every other navigation here
 // already waits for boot; this one did not.
 await page.waitForFunction(()=>document.getElementById('root')?.childElementCount>0);
 // Apply persisted settings before reload: independent switches and migration.
 for(const [claude,codex,place] of [[true,false,'strip'],[false,true,'strip'],[false,false,'strip'],[true,true,'off'],[true,true,'rail']]) {
  await page.evaluate(s=>sessionStorage.setItem('wt-mock-ui-state',JSON.stringify(s)),{usage_claude:claude,usage_codex:codex,usage_place:place});
  await page.reload(); await page.bringToFront(); await page.waitForTimeout(350);
  const calls=await page.evaluate(()=>window.__planUsageCalls??{});
  assert.equal(!!calls.codex_usage,codex&&place!=='off');assert.equal(!!calls.claude_usage,claude&&place!=='off');
  if(place==='off'||(!claude&&!codex))assert.equal(await page.locator('[data-testid="usage-meter"]').count(),0);
  if(place==='rail'){assert(await page.locator('.usage-trig.tile').count());await page.locator('.usage-trig.tile').click();assert.equal(await page.locator('.usage-provider-detail').count(),2);}
  results.push({engine,settings:{claude,codex,place},calls});
 }
 assert.deepEqual(errors,[]);
 await browser.close();
}
for(const [engine,type] of Object.entries({chromium,webkit})) {
 const browser=await type.launch();const page=await browser.newPage({viewport:{width:1100,height:650}});
 await page.goto(baseURL+'/?codexUsage=multi');await page.bringToFront();
 await page.waitForFunction(()=>document.querySelector('.usage-trig')?.textContent.includes('48%'));
 for(const width of [600,450,300]) {
  await page.locator('.usage-strip').evaluate((e,w)=>e.style.width=`${w}px`,width);
  const m=await page.locator('.usage-trig').evaluate(e=>({width:e.getBoundingClientRect().width,labels:[...e.querySelectorAll('.usage-window-label')].map(n=>n.getBoundingClientRect().width),bars:[...e.querySelectorAll('.usage-bar')].map(n=>getComputedStyle(n).display),minimal:getComputedStyle(e.querySelector('.usage-minimal')).display}));
  assert(m.width<width);if(width<=600)assert(m.bars.every(d=>d==='none'));if(width<=450)assert(m.labels.every(w=>w===0));if(width===300)assert(m.minimal!=='none');
  results.push({engine,hostWidth:width,...m});
 }
 await page.locator('.usage-strip').evaluate(e=>e.style.width='');
 for(const settings of [{usage_place:'footer'},{usage_place:'rail',places_side:'left'},{usage_place:'rail',places_side:'right'}]) {
  await page.evaluate(s=>sessionStorage.setItem('wt-mock-ui-state',JSON.stringify({...s,nav_pinned:true})),settings);
  await page.reload();await page.bringToFront();
  if(settings.usage_place==='footer'){ if(!await page.locator('li.row[data-slug]:visible').count()) await page.locator('[data-testid="places-rail"]').click(); await page.locator('li.row[data-slug]:visible').first().click(); }
  await page.waitForFunction(()=>document.querySelector('.usage-trig')?.textContent.includes('48%') || document.querySelector('.usage-trig.tile'));
  if(settings.usage_place==='footer') {
   for(const width of [1280,900,600]) {
    await page.setViewportSize({width,height:650});
    const footer=await page.locator('.statusbar').evaluate(e=>{
     const cs=getComputedStyle(e),r=e.getBoundingClientRect(),b=e.querySelector('.usage-trig').getBoundingClientRect();
     const windows=[...e.querySelectorAll('.usage-window')].filter(n=>n.getBoundingClientRect().width>0);
     return {hostWidth:r.width-parseFloat(cs.paddingLeft)-parseFloat(cs.paddingRight),buttonWidth:b.width,
      contained:b.x>=r.x && b.right<=r.right,windows:windows.map(n=>n.textContent),
      reachable:windows.every(n=>{const r=n.getBoundingClientRect();return e.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));})};
    });
    assert(footer.contained && footer.reachable);
    assert.equal(footer.windows.length,footer.hostWidth<=360?0:footer.hostWidth<=900?2:6);
    results.push({engine,footerViewport:width,footer});
   }
   await page.setViewportSize({width:1100,height:650});
  }
  await page.waitForSelector('.usage-trig');await page.locator('.usage-trig').click();
  await page.waitForSelector('.usage-pop.pinned');
  const bounds=await page.locator('.usage-pop').evaluate(e=>{const r=e.getBoundingClientRect();return {x:r.x,y:r.y,right:r.right,bottom:r.bottom,width:r.width};});
  assert(bounds.x>=0&&bounds.right<=1100&&bounds.y>=0&&bounds.bottom<=650);
  // Real pointerdown inside the panel must keep it pinned; outside must dismiss.
  await page.locator('[data-provider="codex"] strong').click();assert.equal(await page.locator('.usage-pop.pinned').count(),1);
  await page.mouse.click(1090,10);assert.equal(await page.locator('.usage-pop.pinned').count(),0);
  await page.locator('.usage-trig').focus();await page.keyboard.press('Enter');assert.equal(await page.locator('.usage-pop.pinned').count(),1);
  await page.keyboard.press('Escape');assert.equal(await page.locator('.usage-pop.pinned').count(),0);
  results.push({engine,settings,bounds,pointerAndKeyboard:'passed'});
 }
 // Largest UI text at the app minimum window size: final row stays reachable.
 await page.evaluate(()=>sessionStorage.setItem('wt-mock-ui-state',JSON.stringify({usage_place:'strip',ui_rem:22})));
 await page.goto(baseURL+'/?codexUsage=many');await page.bringToFront();await page.setViewportSize({width:900,height:560});
 await page.waitForSelector('.usage-trig');await page.locator('.usage-trig').click();
 await page.waitForFunction(()=>document.querySelector('.usage-pop')?.textContent.includes('Additional quota 11'));
 const scrolls=await page.locator('.usage-pop').evaluate(e=>e.scrollHeight>e.clientHeight);assert(scrolls);
 await page.locator('.usage-pop').evaluate(e=>e.scrollTop=e.scrollHeight);
 const reachable=await page.locator('.usage-pop .usage-row').last().evaluate(e=>{const r=e.getBoundingClientRect();return e.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));});
 assert(reachable);results.push({engine,shortViewportLastRowReachable:reachable});
 // Settings checkboxes actually save explicit false (not just injected fixtures).
 await page.setViewportSize({width:1280,height:800});await page.keyboard.press('Escape');
 await page.keyboard.press('Meta+,');
 const checkbox=page.getByLabel('Codex plan usage',{exact:true});
 await checkbox.waitFor();await checkbox.uncheck();await page.waitForTimeout(500);
 const saved=await page.evaluate(()=>JSON.parse(sessionStorage.getItem('wt-mock-ui-state')));
 assert.equal(saved.usage_codex,false);results.push({engine,checkboxPersistsFalse:true});
 await browser.close();
}
if(process.env.USAGE_REPORT) fs.writeFileSync(process.env.USAGE_REPORT,JSON.stringify(results,null,2));
console.log(JSON.stringify(results.map(({contrast,...result})=>result),null,2));
console.log("plan-usage-browser: both engines passed");
