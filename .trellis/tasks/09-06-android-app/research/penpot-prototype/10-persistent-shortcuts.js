// v5 planning revision: one scrollable terminal, persistent keys, OS photo handoff.
const D=storage.zd,C=D.C;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49')throw new Error('Wrong page');
if(D.boards.terminal.children.some(s=>s.name==='Persistent shortcut surface'))throw new Error('v5 already applied');
D.links=[];
const termKeys=['terminal','keyboard','history','selection','extended','copied','recover','newTerminal','devTerminal','deployTerminal','renamedTerminal','closed'];
for(const key of termKeys){
  const b=D.boards[key];
  if(key==='keyboard')continue; // Existing 44 px row already sits directly above IME.
  // Keep the visible grid above the fixed row without covering output or tap targets.
  for(const s of [...b.children]){
    if(s.name.startsWith('Terminal row / ')&&s.y-b.y>=754)s.remove();
    if(s.name==='Touch / Simulate swipe')s.resize(390,589);
    if(s.name==='Touch / Terminal input'){s.y=b.y+680;s.resize(390,74);}
    if(s.name==='Touch / Simulate scroll to bottom'){s.y=b.y+699;s.resize(390,55);}
    if(s.name==='Touch / Dismiss selection')s.resize(390,667);
    if(s.name==='Disconnected content')s.resize(390,668);
  }
  // Preserve the newest prompt at the bottom when reducing the live viewport.
  if(['terminal','renamedTerminal','recover'].includes(key)){
    const rows=b.children.filter(s=>s.name.startsWith('Terminal row / ')).sort((a,z)=>a.y-z.y);
    const lines=D.v2rows.slice(-rows.length);
    rows.forEach((s,i)=>{s.characters=lines[i][0];s.fills=[{fillColor:C[lines[i][1]],fillOpacity:1}];});
  }
  D.rect(b,0,754,390,70,C.bg,0,'Persistent shortcut surface');
  D.rect(b,8,755,374,1,C.line,0,'Shortcut divider');
  const row=D.shortcuts(b,764);row.resize(374,44);row.x=b.x+8;row.flex.columnGap=3;
  for(const k of row.children){k.resize(44.125,44);k.children[0].resize(44.125,44);}
  if(key==='recover'||key==='closed')row.opacity=0.35;
}
D.boards.terminal.name='04 · 终端 / 底部';
D.boards.history.name='09 · 终端 / 向上滚动';
D.boards.copied.name='17 · 终端 / 复制后';

// Remove the app-owned gallery drawing. This board is an editor-only OS handoff note.
const a=D.boards.album;for(const s of [...a.children])s.remove();
a.showInViewMode=false;a.name='系统交互说明 / 图片选择器';a.resize(390,250);
a.fills=[{fillColor:C.panel,fillOpacity:1}];
D.text(a,24,20,'Android 系统图片选择器',20,C.text,342,38,600);
D.text(a,24,78,'相册入口 → 系统选择单张图片\n返回图片 → 识别配对二维码\n取消 → 回到扫码',15,C.muted,342,98);
D.text(a,24,192,'设计注释 · 此处不实现 App 相册页面',12,C.dim,342,34);
const albumHit=penpotUtils.findShape(s=>s.name==='Touch / Album',D.boards.scanner);
for(const i of [...albumHit.interactions])i.remove();
albumHit.name='Touch / Simulate system Photo Picker result';
albumHit.addInteraction('click',{type:'navigate-to',destination:D.boards.paired});
const guide=penpotUtils.findShape(s=>s.name==='Prototype guide',penpot.root);
const note=penpotUtils.findShape(s=>s.type==='text'&&s.characters.startsWith('①'),guide);
note.characters='① 首页 → 终端；标题 → 会话列表；每行 ··· → 重命名 / 删除\n\n② 同一个终端：文字点击模拟上下滚动 / 长按；手柄模拟延伸选区\n\n③ 快捷键常驻；点底部输入区模拟键盘；系统箭头收起\n\n④ 相册交给 Android 系统；此原型点击相册直接模拟系统返回图片后的配对结果';
for(const f of penpot.currentPage.flows.filter(f=>/^Flow \d+$/.test(f.name)))f.remove();
penpot.selection=[D.boards.terminal];penpot.viewport.zoomIntoView([D.boards.terminal,D.boards.history]);
return {revision:5,terminalStates:termKeys.length,album:a.showInViewMode};
