const fs = require('node:fs');
const assert = require('node:assert/strict');
const { chromium } = require('/home/lucas/Developer/personal/cutokyo-community/ui/node_modules/@playwright/test');
const input = JSON.parse(fs.readFileSync('/tmp/cutokyo-keyboard-isolation-20261004/inputs.json','utf8'));
const out = '/home/lucas/Developer/personal/cutokyo-community/.claude/fleets/20260919-cutokyo-v01/evidence/product-audit-20261004/native-keyboard-diagnosis';
fs.mkdirSync(out, {recursive:true});
(async()=>{
const browser = await chromium.launch({headless:true});
const page = await browser.newPage();
const results = [];
try {
for(let round=0;round<5;round++) {
for(const mode of ['embedded-driver-script','trusted-keyboard']) {
await page.setContent('<a class="skip-link" href="#main-content">Skip to content</a><main id="main-content" tabindex="-1"><h1>Sessions</h1></main>');
await page.evaluate(handler=>{
window.eventLog=[];
for(const type of ['keydown','keyup','click','focusin']) document.addEventListener(type,e=>window.eventLog.push({type:e.type,key:e.key??null,trusted:e.isTrusted,tag:e.target.tagName,id:e.target.id,class:e.target.className}),true);
document.querySelector('.skip-link').addEventListener('click',new Function('event',handler));
},input.click_handler);
if(mode==='embedded-driver-script') {
await page.evaluate(()=>document.querySelector('.skip-link').focus());
await page.evaluate(input.driver_scripts.keydown);
await page.waitForTimeout(10);
await page.evaluate(input.driver_scripts.keyup);
} else {
await page.keyboard.press('Tab');
assert.equal(await page.evaluate(()=>document.activeElement.className),'skip-link');
await page.keyboard.press('Enter');
}
const actual=await page.evaluate(()=>({id:document.activeElement.id,tag:document.activeElement.tagName,class:document.activeElement.className,hash:location.hash,events:window.eventLog}));
results.push({round,mode,expectedActiveId:'main-content',actual,acceptance:actual.id==='main-content'?'PASS':'FAIL'});
if(mode==='embedded-driver-script') assert.equal(actual.id,'', 'red reproduction of empty activeElement.id must remain detectable');
else assert.equal(actual.id,'main-content');
}
}
const record={scope:'Serverless Chromium minimal isolation of exact embedded Enter scripts and exact AppShell click handler; NOT native Tauri acceptance or a full app run',sources:{driver:input.driver_source,appShell:input.app_shell_source},viewport:page.viewportSize(),repetitions:5,results};
fs.writeFileSync(out+'/isolation-results.json',JSON.stringify(record,null,2)+'\n');
console.log(JSON.stringify({mode:'isolated differential loop',embeddedFailures:results.filter(x=>x.mode==='embedded-driver-script'&&x.acceptance==='FAIL').length,trustedPasses:results.filter(x=>x.mode==='trusted-keyboard'&&x.acceptance==='PASS').length,firstEmbedded:results[0],firstTrusted:results[1],evidence:out+'/isolation-results.json'},null,2));
} finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
