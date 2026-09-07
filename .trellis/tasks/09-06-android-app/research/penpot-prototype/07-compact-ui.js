// Incremental v2 design revision. Existing board IDs and incoming links survive.
// Requires the original helper definitions in storage.zd. Run once.
const D = storage.zd, C = D.C;
if (penpot.currentPage.id !== 'c828d3cf-7d4e-8145-8008-98f4fa037d49') throw new Error('Wrong page');
if (D.boards.settings) throw new Error('v2 already applied');
D.links = [];
D.icons.settings = 'M9 3h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1zM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0';
D.icons.sessions = 'M4 4h16v5H4zM4 12h16v8H4z';
function clear(b) { for (const s of [...b.children]) s.remove(); return b; }
function reset(key) { const b = clear(D.boards[key]); D.status(b); D.gesture(b); return b; }
function head(b, title, back='home') {
  D.icon(b,'back',16,49,22); D.hit(b,4,37,48,48,back,'Back');
  D.text(b,58,45,title,18,C.text,270,30,600);
}
function navIcon(b,key,x,y,target,name,kind='nav') {
  D.icon(b,key,x+12,y+12,24,C.muted); D.hit(b,x,y,48,48,target,name,kind);
}

// Settings destination: only an About value until settings scope is reviewed.
let b = D.screen('settings','27 · 设置',2,7); head(b,'设置');
D.text(b,24,123,'关于',13,C.muted,342,28);
const about=D.box(b,24,165,342,66,C.panel,16,'About');
D.text(about,18,18,'zterm',17,C.text,190,28,600);
D.text(about,228,19,'开发版',13,C.muted,96,26,400,false,'right');

// Home: recent connection is the first content, absent when no prior connection.
function home(b,recent) {
  D.icon(b,'terminal',25,54,27,C.accent); D.text(b,64,51,'zterm',24,C.text,160,34,600);
  navIcon(b,'settings',326,43,'settings','Settings');
  if(recent) {
    const card=D.box(b,24,112,342,136,C.panel,20,'Recent connection');
    D.text(card,20,15,'最近连接',12,C.accent,220,24,500);
    D.text(card,20,45,'MacStudio',23,C.text,260,34,600);
    D.text(card,20,88,'android-app',13,C.muted,240,25,400,true);
    D.rect(card,276,72,46,46,C.accent,23,'Resume'); D.icon(card,'right',288,84,22,C.bg);
    D.link(card,'terminal');
  }
  const y=recent?278:120;
  D.text(b,24,y,'已保存的主机',13,C.muted,280,26,500);
  D.text(b,336,y,'2',13,C.dim,30,26,400,false,'right');
  for(const [i,name,sub,target] of [[0,'MacStudio','已连接 · 3 个会话','terminal'],[1,'build-server','未连接','hostOffline']]) {
    const card=D.box(b,24,y+40+i*90,342,78,C.panel,16,'Host / '+name);
    D.icon(card,'server',17,27,23,i===0?C.accent:C.muted);
    D.text(card,60,13,name,16,C.text,236,28,600);
    D.text(card,60,43,sub,12,C.muted,230,22);
    D.icon(card,'right',303,27,22,C.muted); D.link(card,target,i===1?'overlay':'nav');
  }
  D.button(b,24,748,342,'添加主机','scanner','secondary','plus');
}
b=reset('home'); home(b,true);
b=D.screen('homeNoRecent','28 · 主机 / 无最近连接',3,7); home(b,false);

// Camera-led scanner, with only its title and two explicit alternative entries.
b=reset('scanner'); head(b,'扫码');
D.rect(b,16,107,358,588,'#151E17',20,'Camera preview');
D.rect(b,37,229,316,244,C.panel2,8,'Host screen in camera');
D.rect(b,49,242,292,15,'#344235',3,'Window chrome');
for(let i=0;i<3;i++)D.dot(b,57+i*12,247,4,C.dim);
D.qr(b,108,281,174);
for(const [x,y,dx,dy] of [[61,215,1,1],[329,215,-1,1],[61,495,1,-1],[329,495,-1,-1]]) {
  D.rect(b,dx===1?x:x-24,y,24,2,C.accent);
  D.rect(b,x,dy===1?y:y-24,2,24,C.accent);
}
D.rect(b,63,358,266,1,C.accent,0,'Scan line');
D.hit(b,35,196,320,325,'paired','Simulate scan');
D.icon(b,'album',43,747,25,C.accent); D.text(b,79,744,'相册',15,C.text,76,30,500);
D.hit(b,24,730,140,60,'album','Album');
D.icon(b,'ticket',235,747,25,C.accent); D.text(b,271,744,'填入票据',15,C.text,96,30,500);
D.hit(b,216,730,150,60,'ticket','Ticket');

// Compact pairing form. No explanatory paragraphs or fake input in blank state.
for(const [key,ready] of [['ticket',false],['ticketReady',true]]) {
  b=reset(key); head(b,'票据','scanner');
  D.text(b,24,126,'配对票据',13,C.muted,342,25);
  D.rect(b,24,167,342,136,C.panel,14,'Ticket field');
  D.text(b,40,183,ready?'zterm-pair://\n•••• •••• •••• ••••':'输入或粘贴票据',14,ready?C.text:C.dim,310,93,400,ready);
  if(!ready) {D.hit(b,24,167,342,136,'ticketReady','Simulate input');D.button(b,24,329,342,'粘贴','ticketReady','secondary','copy');}
  D.button(b,24,397,342,'连接',ready?'paired':null,'primary','right').opacity=ready?1:0.35;
}
b=reset('paired'); head(b,'配对'); D.dot(b,157,220,76,'#2C402B'); D.icon(b,'check',178,241,34,C.accent);
D.text(b,24,322,'已连接',24,C.text,342,40,600,false,'center');
D.text(b,24,372,'MacStudio',16,C.muted,342,30,500,false,'center');
D.button(b,24,748,342,'进入终端','terminal','primary','right');

// Terminal is a dense, fixed-row text grid. All explanatory chrome is removed.
D.v2rows=[
  ['$ pwd','text'],['/Users/leon/projects/zterm','blue'],['$ git status --short','text'],
  [' M docs/android.md','amber'],['$ cargo test -p zterm-core','text'],
  ['   Compiling zterm-core v0.1.22','muted'],['    Finished test profile in 1.42s','muted'],
  ['     Running unittests src/lib.rs','muted'],['running 128 tests','text'],
  ...['session::create','session::rename','session::switch','session::close','session::restore',
  'history::append','history::scroll','history::prefetch','history::cache_hit','history::evict',
  'surface::resize','surface::cursor','surface::unicode','surface::soft_wrap','surface::colors',
  'select::start','select::extend','select::reverse','select::unicode','select::cross_page',
  'pair::qr_ticket','pair::expiry','input::modifiers','input::ime','input::arrows',
  'relay::connect','relay::reconnect','lease::takeover','lease::detach'].map(n=>['test '+n.padEnd(28,'.')+' ok','green']),
  ['test result: ok. 128 passed; 0 failed','green'],['$','text']
];
D.v2history=[
  ['$ git log --oneline -12','text'],
  ...['6f13a4e  Refine Android planning','320e9ca  Preserve history row identity',
  '2a9d831  Deduplicate range reads','17ee02b  Guard input on reconnect',
  '9ec12b0  Handle terminal resize','e419088  Keep session lease explicit',
  '938efa0  Retain pinned output','3a216fc  Add semantic cursor state',
  '71698aa  Normalize soft line breaks','27bdf89  Validate pairing ticket',
  '180e226  Add session rename','c14a038  Keep process on detach'].map(s=>[s,'muted']),
  ['$ cargo test -p zterm-core','text'],...D.v2rows.slice(5)
];
function terminalHead(p,session,kind) {
  D.icon(p,'back',12,47,22,C.muted);D.hit(p,0,35,48,48,'home','Back to hosts');
  D.text(p,49,37,session,15,C.text,224,23,600);
  D.dot(p,50,68,4,kind==='recover'?C.amber:C.green);D.text(p,61,59,'MacStudio',10,C.muted,170,21);
  navIcon(p,kind==='keyboard'?'down':'keyboard',282,35,kind==='keyboard'?'terminal':'keyboard','Keyboard');
  navIcon(p,'more',334,35,'actions','Session menu','overlay');
  D.rect(p,0,85,390,1,C.line,0,'App bar divider');
}
function rows(p,lines,y=91,max=42) {
  for(const [i,[str,color]] of lines.slice(0,max).entries()) {
    const t=D.text(p,8,y+i*17,str,12,C[color],374,17,400,true); t.lineHeight='1.2';t.verticalAlign='top';t.name='Terminal row / '+i;
  }
}
function selectionHandle(p,x,y,target,name) {
  D.rect(p,x,y-12,2,13,C.accent,0,'Selection stem');D.dot(p,x-6,y,14,C.accent);
  D.hit(p,Math.max(0,x-21),y-18,44,48,target,name);
}
for(const key of ['terminal','keyboard','history','selection','extended','copied','recover','newTerminal','devTerminal','deployTerminal','renamedTerminal']) {
  const isKeyboard=key==='keyboard';
  // Reuse native editable IME illustration while rebuilding the surrounding chrome.
  const p=D.boards[key];const ime=key==='keyboard'?penpotUtils.findShape(s=>s.name==='Android system keyboard / illustrative',p):null;
  if(ime)penpot.root.appendChild(ime);
  b=reset(key);
  const session={newTerminal:'scratch',devTerminal:'dev-server',deployTerminal:'deploy',renamedTerminal:'android-work'}[key]||'android-app';
  terminalHead(b,session,key);
  const selected=key==='selection'||key==='extended';const past=['history','selection','extended','copied'].includes(key);
  if(selected)D.rect(b,8,key==='extended'?91:295,370,key==='extended'?510:187,C.select,0,'Selected text');
  let lines=past?D.v2history:D.v2rows;
  if(key==='extended')lines=D.v2history.slice(12);
  if(isKeyboard)lines=D.v2rows.slice(-22);
  if(key==='newTerminal')lines=[['$ pwd','text'],['/Users/leon','blue'],['$','text']];
  if(key==='devTerminal')lines=[['$ npm run dev','text'],['  VITE ready in 284 ms','green'],['  Local: http://localhost:5173/','blue']];
  if(key==='deployTerminal')lines=[['$ ./deploy.sh --status','text'],['web       running','green'],['worker    running','green'],['database  healthy','green'],['$','text']];
  rows(b,lines,91,isKeyboard?22:42);
  const secondary=['newTerminal','devTerminal','deployTerminal','renamedTerminal'].includes(key);
  if(secondary) {
    // Only Back and the owning session list are demonstrated for secondary endpoints.
    D.links=D.links.filter(l=>l.shape.parent?.id!==b.id||l.shape.name==='Touch / Back to hosts');
    const menu=penpotUtils.findShape(s=>s.name==='Touch / Session menu',b);
    D.link(menu,key==='renamedTerminal'?'renamed':'sessions');
  } else if(isKeyboard) {
    const shortcut=D.shortcuts(b,478);shortcut.resize(374,44);shortcut.x=b.x+8;shortcut.flex.columnGap=3;
    for(const k of shortcut.children){k.resize(44.125,44);k.children[0].resize(44.125,44);}
    b.appendChild(ime);ime.x=b.x;ime.y=b.y+526;
  } else if(selected) {
    D.hit(b,0,87,390,724,'history','Dismiss selection');
    const menuY=key==='extended'?551:246;
    const menu=D.box(b,151,menuY,88,40,C.panel2,20,'Android floating Copy action');
    D.text(menu,0,0,'复制',14,C.text,88,40,500,false,'center');D.link(menu,'copied');
    if(key==='selection')selectionHandle(b,8,295,'extended','Extend start');
    selectionHandle(b,376,key==='extended'?601:482,'extended','Extend end');
  } else if(past) {
    D.hit(b,0,91,390,604,'selection','Simulate long press');
    D.hit(b,0,699,390,115,'terminal','Simulate scroll to bottom');
  } else if(key==='recover') {
    const shade=D.rect(b,0,86,390,734,C.bg,0,'Disconnected content');shade.opacity=0.65;
    D.rect(b,83,365,224,98,C.panel,16,'Reconnect');D.text(b,101,377,'连接中…',16,C.text,145,40,500);
    D.text(b,101,421,'重试',14,C.accent,172,27,500);D.hit(b,83,412,224,49,'terminal','Retry');
  } else {
    D.hit(b,0,91,390,632,'history','Simulate swipe');D.hit(b,0,727,390,86,'keyboard','Terminal input');
    D.hit(b,45,61,179,21,'recover','Simulate interruption');
  }
  if(past){D.rect(b,386,188,2,78,C.dim,1,'Transient scroll thumb');}
}

// Session management stays available on demand, not between host and terminal.
b=clear(D.boards.actions);b.resize(390,396);D.rect(b,175,12,40,4,C.line,2,'Sheet handle');
D.text(b,24,29,'android-app',18,C.text,290,30,600);
navIcon(b,'close',326,22,null,'Close menu','close');
for(const [i,label,icon,target,kind] of [
  [0,'会话','sessions','sessions','nav'],[1,'选择文本','copy','selection','nav'],
  [2,'历史','history','history','nav'],[3,'重命名','edit','rename','overlay'],
  [4,'断开','disconnect','home','nav'],[5,'结束会话','trash','close','overlay']
]) {const y=81+i*48;D.icon(b,icon,26,y+9,22,i===5?C.red:C.muted);D.text(b,63,y+4,label,15,i===5?C.red:C.text,270,32,500);D.hit(b,12,y,366,48,target,label,kind);}

// Remove instructional copy elsewhere; preserve concise error/consequence text.
const omit=new Set(['切换会话，工作会继续运行。','已连接 · 连接状态良好','系统照片选择器','只会读取你选择的图片。','在 MacStudio 上开始一段新工作。','修改名称不会重启会话或中断工作。']);
const replacements={
  '选择二维码图片':'选择图片','留空使用主机默认目录':'主机默认目录','创建并进入':'创建','保存名称':'保存',
  '结束这个会话？':'结束会话？','这会终止主机上的会话和正在运行的进程。\n此操作无法恢复。':'将终止会话及其中的进程。',
  '会话正在电脑上使用':'会话已被占用','接管后，电脑将失去控制权。\n会话中的程序会继续运行。':'接管将断开电脑控制，程序继续运行。',
  '暂时无法连接主机':'无法连接','请确认主机在线，网络连接正常，\n然后再次尝试。':'请检查主机和网络。'
};
for(const p of Object.values(D.boards))for(const s of penpotUtils.findShapes(s=>s.type==='text',p)) {
  if(omit.has(s.characters))s.remove();else if(replacements[s.characters]){s.characters=replacements[s.characters];s.name=s.characters;}
}
// Keep the guide outside the app UI; make prototype-only click simulation explicit.
const guide=penpotUtils.findShape(s=>s.name==='Prototype guide',penpot.root);
const note=penpotUtils.findShape(s=>s.type==='text'&&s.characters.startsWith('①'),guide);
note.characters='① 首页：最近连接 / 设备 → 终端；齿轮 → 设置\n\n② 终端：点击内容模拟滚动 → 点击文字模拟长按 → 点击手柄模拟跨屏延伸 → 复制\n\n③ 终端右上角 ··· → 会话 / 重命名 / 断开 / 结束\n\n④ 扫码：点击取景区；或左下相册 / 右下票据';

// Existing interactions elsewhere remain; all newly created shapes are wired here.
for(const l of D.links) {
  if(l.kind!=='close'&&!D.boards[l.target])throw new Error('Missing '+l.target);
  const dest=D.boards[l.target];
  l.shape.addInteraction('click',l.kind==='close'?{type:'close-overlay'}:l.kind==='overlay'?{type:'open-overlay',destination:dest,position:dest.width===390?'bottom-center':'center',closeWhenClickOutside:true,addBackgroundOverlay:true}:{type:'navigate-to',destination:dest});
}
for(const f of penpot.currentPage.flows.filter(f=>/^Flow \d+$/.test(f.name)))f.remove();
penpot.selection=[D.boards.home];penpot.viewport.zoomIntoView([D.boards.home,D.boards.scanner,D.boards.terminal]);
return {settings:D.boards.settings.id,noRecent:D.boards.homeNoRecent.id,newLinks:D.links.length};
