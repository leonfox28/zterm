const D=storage.zd,C=D.C;
// Keep labels and rendered text inside their boards.
for(const s of penpotUtils.findShapes(s=>s.type==='text'&&s.characters.startsWith('────'),penpot.root)){s.characters='────────────────────────';s.name='Terminal separator';}
for(const s of penpot.root.children.filter(s=>s.type==='text'))s.fills=[{fillColor:s.fontSize==='34'?'#203529':'#607668',fillOpacity:1}];
for(const row of penpotUtils.findShapes(s=>s.name==='Terminal shortcuts / fixed eight keys',penpot.root)){
 const p=row.parent;row.resize(374,44);row.x=p.x+8;row.flex.columnGap=3;
 for(const key of row.children){key.resize(44.125,44);key.children[0].resize(44.125,44);}
}
const sel=D.boards.selection;
const label=penpotUtils.findShape(s=>s.type==='text'&&s.characters==='已选 24 行',sel);label.characters='已选 12 行';label.name='已选 12 行';
const region=penpotUtils.findShape(s=>s.name==='Selected semantic text',sel);region.y=sel.y+389;region.resize(353,264);
const dots=penpotUtils.findShapes(s=>s.type==='ellipse'&&s.width===14,sel);dots[0].y=sel.y+382;dots[1].y=sel.y+646;
const startHit=penpotUtils.findShape(s=>s.name==='Touch / Extend selection across screens',sel);startHit.y=sel.y+366;
const endHit=penpotUtils.findShape(s=>s.name==='Touch / Extend selection handle',sel);endHit.y=sel.y+630;
// The pasted-ticket state makes the input simulation explicit.
let b=D.screen('ticketReady','26 · 票据已粘贴',1,6);D.nav(b,'填入票据','无需使用相机','scanner');D.text(b,24,156,'粘贴主机的配对票据',24,C.text,342,38,600);D.text(b,24,211,'从主机复制完整票据，确认后即可连接。',14,C.muted,342,46);D.field(b,289,'配对票据','zterm-pair://\n•••• •••• •••• ••••\n•••• •••• •••• ••••',null,3);D.icon(b,'lock',25,490,18,C.dim);D.text(b,53,486,'票据仅用于本次配对，请勿转发。',12,C.muted,313,30);D.icon(b,'check',25,597,21,C.accent);D.text(b,60,593,'已粘贴票据',14,C.accent,270,30,500);D.button(b,24,660,342,'确认连接','paired','primary','right');
const ticket=D.boards.ticket;const entry=penpotUtils.findShape(s=>s.type==='text'&&s.characters.startsWith('zterm-pair://'),ticket);entry.characters='在此填入完整的配对票据';entry.name='Empty pairing ticket';D.fonts.cn.applyToText(entry);entry.fontSize='14';entry.fills=[{fillColor:C.dim,fillOpacity:1}];
const pasteButton=penpotUtils.findShape(s=>s.name==='Button / 粘贴票据',ticket);D.link(pasteButton,'ticketReady');D.hit(ticket,24,319,342,120,'ticketReady','Simulate ticket entry');
const confirm=penpotUtils.findShape(s=>s.name==='Button / 确认连接',ticket);confirm.opacity=0.35;D.links=D.links.filter(l=>l.shape.id!==confirm.id);
// Secondary destinations are endpoints for the first prototype. Only their
// Back control is wired; keep unimplemented actions from switching identity.
for(const key of ['newTerminal','devTerminal','deployTerminal','renamedTerminal']){const p=D.boards[key];D.links=D.links.filter(l=>l.shape.parent?.id!==p.id||l.shape.name==='Touch / Back');const back=D.links.find(l=>l.shape.parent?.id===p.id&&l.shape.name==='Touch / Back');if(key==='renamedTerminal')back.target='renamed';}
D.hit(D.boards.terminal,16,126,155,38,'recover','Simulate connection interruption');
// Resolve all destinations before changing interactions.
for(const l of D.links)if(l.kind!=='close'&&!D.boards[l.target])throw new Error('Missing destination '+l.target);
for(const l of D.links){const destination=D.boards[l.target];const action=l.kind==='close'?{type:'close-overlay'}:l.kind==='overlay'?{type:'open-overlay',destination,position:destination.width===390?'bottom-center':'center',closeWhenClickOutside:true,addBackgroundOverlay:true}:{type:'navigate-to',destination};l.shape.addInteraction('click',action);}
for(const [name,key] of [['完整流程 · 从主机开始','home'],['配对 · 三种入口','scanner'],['终端 · 输入与跨屏复制','terminal']])penpot.currentPage.createFlow(name,D.boards[key]);
const note=D.box(null,960,5920,870,500,C.panel,24,'Prototype guide');note.showInViewMode=false;
D.text(note,32,26,'原型怎么试',28,C.text,790,45,600);D.text(note,32,91,'① 主机 → 继续会话 → 历史记录 → 点击文字选区 → 点击手柄跨屏延伸 → 复制\n\n② 添加主机 → 扫码区域 / 左下角相册 / 右下角票据 → 配对成功\n\n③ 会话列表 → 新建；android-app 的 ··· → 重命名 / 结束会话\n\n④ 点击终端「已连接」模拟网络中断，再点重试返回实时。',16,C.muted,802,280);
D.text(note,32,394,'原型以点击模拟扫描、输入、长按及拖动；\n表单、键盘、复制提示均为演示状态，不会连接真实主机或写入系统剪贴板。',13,C.dim,802,65);
penpot.selection=[D.boards.home];penpot.viewport.zoomIntoView([D.boards.home,D.boards.scanner,D.boards.sessions,D.boards.terminal]);
return {boards:Object.fromEntries(Object.entries(D.boards).map(([k,b])=>[k,{id:b.id,name:b.name}])),interactions:D.links.length,flows:penpot.currentPage.flows.map(f=>({name:f.name,start:f.startingBoard.id}))};
