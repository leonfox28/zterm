// v8b: landscape terminal and local saved-device removal walkthrough.
const D=storage.zd,C=D.C,V=D.v8;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49'||!V?.fonts)throw new Error('Wrong page/order');
if(D.boards.landscape)throw new Error('v8b already applied');
const b=D.box(null,1640,18000,844,390,C.bg,24,'终端 / 横屏');b.showInViewMode=true;D.boards.landscape=b;
D.status(b);
for(const s of b.children)if(s.x-b.x>=300)s.x+=454;
D.icon(b,'back',12,47,22);V.nav(V.hit(b,0,35,48,48,'Back to hosts'),D.boards.home);
D.text(b,49,37,'android-app',16,C.text,230,23,600);D.text(b,49,60,'MacStudio',11,C.muted,230,20);
D.icon(b,'down',149,41,18,C.muted);D.rect(b,0,85,844,1,C.line,0,'App bar divider');
const lines=[
 ['$ git diff --stat',C.text],
 [' app/src/main/java/dev/zterm/TerminalView.kt | 24 ++++++++++++++++++++----',C.muted],
 [' app/src/main/java/dev/zterm/Settings.kt     | 18 ++++++++++++++++++',C.muted],
 [' 2 files changed, 38 insertions(+), 4 deletions(-)',C.muted],
 ['$ cargo test -p zterm-core',C.text],
 ['   Compiling zterm-core v0.1.22',C.muted],
 ['    Finished test profile in 1.42s',C.green],
 ['running 128 tests',C.text],
 ['test history::preserve_anchor_after_resize ........................... ok',C.green],
 ['test input::resume_first_character_once .............................. ok',C.green],
 ['test input::discard_after_reconnect .................................. ok',C.green],
 ['test result: ok. 128 passed; 0 failed; 0 ignored',C.green],
 ['$ ',C.text]];
lines.forEach(([line,color],i)=>{const t=D.text(b,8,91+i*17,line,12,color,828,17,400,true);t.name='Terminal row / landscape '+i;});
D.rect(b,0,314,844,62,C.bg,0,'Persistent shortcut surface');D.rect(b,8,315,828,1,C.line,0,'Shortcut divider');
const keys=D.shortcuts(b,322);keys.x=b.x+8;keys.resize(828,44);keys.flex.columnGap=3;
for(const key of keys.children){key.resize(100.875,44);key.children[0].resize(100.875,44);}
D.rect(b,360,376,124,4,C.muted,2,'System gesture handle');
// Same title list in landscape, with enough height left for the fixed keys.
const panel=V.clone('landscapeSessions','sessions','横屏 / 标题展开',1640,18500);panel.resize(390,312);
for(const s of panel.children){if(s.name==='Expanded app bar')s.resize(390,226);if(s.y-panel.y>=267)s.y-=14;}
V.open(V.hit(b,48,35,280,50,'Expand sessions'),panel,'top-center');

const removed=V.clone('homeRemoved','homeNoRecent','主机 / 已移除 MacStudio',0,19800);
for(const s of [...removed.children]){if(s.name==='Host / MacStudio')s.remove();if(s.name==='Host / build-server')s.y=removed.y+160;if(s.type==='text'&&s.characters==='2'){s.characters='1';s.name='1';}}
const removal=V.modal('removeDevice','移除设备 / MacStudio',420,19800,342,228);
D.text(removal,24,24,'移除 MacStudio？',20,C.text,294,34,600);
D.text(removal,24,79,'仅移除此手机上的记录。',14,C.muted,294,46);
V.button(removal,24,154,141,'取消',C.panel2,C.text).addInteraction('click',{type:'close-overlay',destination:removal});
V.nav(V.button(removal,177,154,141,'移除',C.accent,C.bg),removed);
const hold=V.clone('homeRemovalDemo','home','主机 / 长按设备示例',840,19800);
hold.addInteraction('after-delay',{type:'open-overlay',destination:removal,position:'center',closeWhenClickOutside:true,addBackgroundOverlay:true},400);
const empty=V.clone('homeEmpty','homeNoRecent','主机 / 尚未添加',1260,19800);
for(const s of [...empty.children])if(s.name.startsWith('Host / ')||(s.type==='text'&&['2','已保存的主机'].includes(s.characters)))s.remove();
D.text(empty,24,362,'暂无主机',16,C.muted,342,32,400,false,'center');
for(const [name,start] of [['终端 · 横屏',b],['设备 · 长按移除',hold],['首页 · 尚未添加',empty]])penpot.currentPage.createFlow(name,start);
for(const f of penpot.currentPage.flows.filter(f=>/^Flow \d+$/.test(f.name)))f.remove();
return {landscape:b.id,removal:removal.id,removed:removed.id};
