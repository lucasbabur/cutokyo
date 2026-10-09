import { createRequire } from 'node:module';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { strict as assert } from 'node:assert';
const repository = '/home/lucas/Developer/personal/cutokyo-green-theme';
const evidence = '/home/lucas/Developer/personal/cutokyo-community/.claude/fleets/20260919-cutokyo-v01/evidence/green-shell-review-20260929';
const require = createRequire(repository + '/ui/package.json');
const { chromium, expect } = require('@playwright/test');
const { AxeBuilder } = require('@axe-core/playwright');
const browser = await chromium.launch({headless:true});
const states = [
 ['dashboard','populated-dashboard','/dashboard','Overview'],
 ['onboarding','onboarding-empty-history','/onboarding','See your agent work without sending it away'],
 ['empty-history','empty-history','/sessions','Sessions'],
 ['settings','visual-keyboard-consistency','/settings','Settings'],
 ['health','degraded-health-recovery','/health','System health'],
 ['resume','search-detail-resume','/sessions/session-claude-73A9','Reconcile usage capture'],
 ['analysis','analysis-preview-cancel','/analysis','AI analysis'],
 ['inventory','mcp-plugin-inventory','/inventory','Agent inventory'],
 ['guards','guard-proxy-coverage-language','/guards','Guards & capture'],
];
const sizes = [{width:900,height:700},{width:1280,height:800},{width:1536,height:960}];
const records=[];
mkdirSync(evidence+'/screens-diagnostic',{recursive:true});
function save() {writeFileSync(evidence+'/browser-diagnostic.json',JSON.stringify({revision:JSON.parse(readFileSync(evidence+'/environment.json','utf8')).revision,environment:'fixture-only Vite, Chromium, isolated browser contexts',native:false,records},null,2));}
async function shell(page) {
 const geometry = await page.evaluate(async (asset) => {
  const main=document.getElementById('main-content'); const workspace=document.querySelector('.workspace'); const img=document.querySelector('.brand img');
  if(!(img instanceof HTMLImageElement)||!main||!workspace) throw Error('Expected shell elements missing');
  const canonical=(xml)=>{const doc=new DOMParser().parseFromString(xml,'image/svg+xml');const walk=(node)=>{for(const child of [...node.childNodes]) {if(child.nodeType===3&&!child.textContent.trim())child.remove();else walk(child)}};walk(doc.documentElement);return new XMLSerializer().serializeToString(doc.documentElement)};
  const image=await (await fetch(img.currentSrc)).text();
  const rect=main.getBoundingClientRect(); const heading=document.querySelector('h1').getBoundingClientRect();
  return {theme:document.documentElement.dataset.theme,mainTop:rect.top,workspaceTop:workspace.getBoundingClientRect().top,headingTop:heading.top,mainPaddingTop:getComputedStyle(main).paddingTop,width:innerWidth,height:innerHeight,documentWidth:document.documentElement.scrollWidth,documentHeight:document.documentElement.scrollHeight,bodyWidth:document.body.scrollWidth,bodyHeight:document.body.scrollHeight,logo:{tag:img.tagName,alt:img.getAttribute('alt'),complete:img.complete,naturalWidth:img.naturalWidth,naturalHeight:img.naturalHeight,localAssetMatch:canonical(image)===canonical(asset),src:img.currentSrc,adjacentName:img.nextElementSibling?.textContent},obsoleteNodes:document.querySelectorAll('.topbar,.brand__subtitle,.brand small,.sidebar__mode').length,sidebarText:document.querySelector('.sidebar').textContent,workspaceRows:getComputedStyle(workspace).gridTemplateRows,canvas:getComputedStyle(workspace).backgroundColor,sidebar:getComputedStyle(document.querySelector('.sidebar')).backgroundColor};
 },readFileSync(repository+'/ui/src/assets/cutokyo-mark.svg','utf8'));
 assert.equal(geometry.obsoleteNodes,0);assert.equal(geometry.logo.tag,'IMG');assert.equal(geometry.logo.alt,'');assert.ok(geometry.logo.complete&&geometry.logo.naturalWidth>0);assert.equal(geometry.logo.adjacentName,'Cutokyo');assert.equal(geometry.logo.localAssetMatch,true);
 for(const removed of ['Local writer','Local only','Proxy off','AI egress on confirmation only','Local observability'])assert.ok(!geometry.sidebarText.includes(removed),removed+' still in shell');
 assert.ok(geometry.mainTop<=1);assert.ok(geometry.workspaceTop<=1);assert.ok(geometry.headingTop<80);
 assert.ok(geometry.documentWidth<=geometry.width&&geometry.bodyWidth<=geometry.width);assert.ok(geometry.documentHeight<=geometry.height&&geometry.bodyHeight<=geometry.height);
 return geometry;
}
async function keyboard(page,target){
 let found=false;for(let i=0;i<60;i++){await page.keyboard.press('Tab');if(await target.evaluate(e=>e===document.activeElement)){found=true;break;}}
 assert.ok(found,'Target not reachable using Tab');
 await expect(target).toHaveCSS('outline-offset','3px');
 await page.waitForFunction(()=>{const e=document.activeElement,s=getComputedStyle(e),r=e.getBoundingClientRect(),extent=parseFloat(s.outlineWidth)+parseFloat(s.outlineOffset);return r.left-extent>=0&&r.top-extent>=0&&r.right+extent<=innerWidth&&r.bottom+extent<=innerHeight});
 const result=await target.evaluate(e=>{const s=getComputedStyle(e),r=e.getBoundingClientRect(),extent=parseFloat(s.outlineWidth)+parseFloat(s.outlineOffset);const clips=[];for(let a=e.parentElement;a;a=a.parentElement){const style=getComputedStyle(a),p=a.getBoundingClientRect();if(/auto|scroll|hidden|clip/.test(style.overflowX)&&(r.left-extent<p.left||r.right+extent>p.right))clips.push(a.className+':x');if(/auto|scroll|hidden|clip/.test(style.overflowY)&&(r.top-extent<p.top||r.bottom+extent>p.bottom))clips.push(a.className+':y');}return {focusVisible:e.matches(':focus-visible'),outlineWidth:s.outlineWidth,outlineOffset:s.outlineOffset,outlineStyle:s.outlineStyle,clips,rect:{top:r.top,bottom:r.bottom,left:r.left,right:r.right}}});
 assert.equal(result.focusVisible,true);assert.equal(result.outlineStyle,'solid');assert.ok(parseFloat(result.outlineWidth)>=2);assert.ok(parseFloat(result.outlineOffset)>=2);assert.deepEqual(result.clips,[]);return result;
}
try {
 for(const theme of ['light','dark']) for(const size of sizes) for(const [name,scenario,route,heading] of states){
  const record={key:`${name}-${theme}-${size.width}x${size.height}`,viewport:size,steps:[`open fixture ${scenario} at ${route}`,`emulate saved System OS ${theme}`],expected:'loaded official local logo, no removed shell chrome/54px gap, no viewport or focus clipping, zero Axe violations',status:'running'};records.push(record);
  const context=await browser.newContext({viewport:size,colorScheme:theme,reducedMotion:'reduce',locale:'en-US',timezoneId:'UTC'});const page=await context.newPage();const errors=[],remote=[];page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>{if(!r.url().startsWith('http://127.0.0.1:4179/')&&!r.url().startsWith('data:')&&!r.url().startsWith('blob:'))remote.push(r.url())});
  try {
   await page.goto(`http://127.0.0.1:4179/?jev_case=${scenario}#${route}`);await page.getByRole('heading',{name:heading,level:1,exact:true}).waitFor();
   await expect(page.locator('h1')).toHaveCSS('color',theme==='light'?'rgb(37, 55, 40)':'rgb(238, 240, 234)');record.geometry=await shell(page);
   let target=page.getByRole('navigation',{name:'Primary navigation'}).getByRole('link',{name:'Overview',exact:true});
   if(name==='resume'){await page.getByRole('button',{name:'Resume',exact:true}).click();const d=page.getByRole('dialog',{name:'Resume this exact native session?'});assert.equal(await d.getByRole('code').textContent(),'claude-jev-73A9');target=d.getByRole('button',{name:'Resume in Claude Code'});record.steps.push('open exact native resume preview; ID claude-jev-73A9');}
   if(name==='analysis'){await page.locator('article').filter({hasText:'analysis-73A9'}).getByRole('button',{name:'Analyze',exact:true}).click();const d=page.getByRole('dialog',{name:'Review AI analysis egress'});await d.waitFor();target=d.getByRole('button',{name:'Cancel — send nothing'});record.steps.push('open redacted analysis consent preview');}
   if(name==='onboarding')target=page.getByRole('checkbox',{name:/I understand/});
   if(name==='settings')target=page.getByRole('button',{name:theme==='light'?'Light':'Dark',exact:true});
   assert.equal(record.geometry.theme,theme);
   assert.equal(record.geometry.canvas,theme==='light'?'rgb(237, 243, 233)':'rgb(17, 20, 17)');assert.equal(record.geometry.sidebar,theme==='light'?'rgb(226, 235, 221)':'rgb(21, 26, 22)');
   record.axe=await new AxeBuilder({page}).analyze();assert.deepEqual(record.axe.violations,[]);assert.ok(record.axe.passes.some(r=>r.id==='color-contrast'));
   if(name==='dashboard'||size.width===900){record.screenshot=evidence+`/screens-diagnostic/${record.key}.png`;await page.screenshot({path:record.screenshot});}
   record.focus=await keyboard(page,target);record.steps.push('reach target using genuine Tab navigation; inspect focus outline and ancestor clipping');
   if(name==='analysis'){await page.keyboard.press('Enter');const audit=await page.evaluate(()=>globalThis.__CUTOKYO_FIXTURE_AUDIT__());assert.equal(audit.outboundAnalysisRequests,0);record.analysisAudit=audit;}
   assert.deepEqual(errors,[]);assert.deepEqual(remote,[]);record.errors=errors;record.remoteRequests=remote;record.status='browser-observed-success';
  }catch(error){record.status='failed';record.error=String(error);record.screenshot=evidence+`/screens-diagnostic/${record.key}-failed.png`;await page.screenshot({path:record.screenshot});}
  await context.close();save();console.log(record.key,record.status,record.error??'');
 }
 for(const theme of ['light','dark']) for(const size of sizes){
  const record={key:`forced-colors-${theme}-${size.width}x${size.height}`,viewport:size,steps:['emulate active forced-colors','open saved System dashboard','inspect selected navigation outline before and after keyboard focus'],expected:'structural current page marker; 3px outer keyboard outline; no clipped focus',status:'running'};records.push(record);
  const context=await browser.newContext({viewport:size,colorScheme:theme,forcedColors:'active',reducedMotion:'reduce'});const page=await context.newPage();
  try {await page.goto('http://127.0.0.1:4179/?jev_case=populated-dashboard#/dashboard');await page.getByRole('heading',{name:'Overview',level:1}).waitFor();const nav=page.getByRole('navigation',{name:'Primary navigation'}).getByRole('link',{name:'Overview',exact:true});assert.equal(await nav.getAttribute('aria-current'),'page');record.selection=await nav.evaluate(e=>({width:getComputedStyle(e).outlineWidth,offset:getComputedStyle(e).outlineOffset,style:getComputedStyle(e).outlineStyle}));assert.deepEqual(record.selection,{width:'1px',offset:'-1px',style:'solid'});record.geometry=await shell(page);record.focus=await keyboard(page,nav);assert.equal(record.focus.outlineWidth,'3px');assert.equal(record.focus.outlineOffset,'3px');record.axe=await new AxeBuilder({page}).analyze();assert.deepEqual(record.axe.violations,[]);record.screenshot=evidence+`/screens-diagnostic/${record.key}.png`;await page.screenshot({path:record.screenshot});record.status='browser-observed-success';}catch(error){record.status='failed';record.error=String(error)}await context.close();save();console.log(record.key,record.status,record.error??'');
 }
}finally{await browser.close();save();}
const failed=records.filter(r=>r.status==='failed');console.log(JSON.stringify({cases:records.length,failed:failed.length}));process.exitCode=failed.length?1:0;
