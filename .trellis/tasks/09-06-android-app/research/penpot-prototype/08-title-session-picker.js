// Incremental v3 planning-only prototype revision; execute once in the saved page.
const D=storage.zd,C=D.C;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49')throw new Error('Wrong page');
if(D.boards.pickerDev)throw new Error('v3 already applied');
D.links=[];
function clear(b){for(const s of [...b.children])s.remove();return b;}
function unlink(s){for(const i of [...s.interactions])i.remove();}
function arrow(b,title,up=false){D.icon(b,up?'up':'down',49+Math.min(218,Math.max(68,title.length*7.7))+6,41,18,C.muted);}
function header(b,title,expanded=false){
  D.status(b);D.icon(b,'back',12,47,22,C.muted);
  D.hit(b,0,35,48,48,expanded?null:'home','Back',expanded?'close':'nav');
  D.text(b,49,37,title,15,C.text,224,23,600);
  D.dot(b,50,68,4,C.green);D.text(b,61,59,'MacStudio',10,C.muted,170,21);
  arrow(b,title,expanded);D.rect(b,0,85,390,1,C.line,0,'App bar divider');
  if(expanded)D.hit(b,48,35,280,50,null,'Collapse sessions','close');
}
const base=[['android-app','terminal'],['dev-server','devTerminal'],['deploy','deployTerminal']];
function picker(b,title,current,items){
  clear(b);b.resize(390,96+items.length*56+62);b.fills=[{fillColor:C.bg,fillOpacity:1}];
  b.showInViewMode=false;b.borderRadius=24;
  header(b,title,true);
  D.rect(b,0,86,390,b.height-86,C.panel,0,'Expanded app bar');
  // Restore divider over the panel; title itself does not change position.
  D.rect(b,0,85,390,1,C.line,0,'Panel divider');
  for(const [i,[name,target]] of items.entries()){
    const y=96+i*56,active=target===current,occupied=name==='deploy'&&!active;
    if(active)D.rect(b,12,y,366,52,C.panel2,12,'Current session');
    D.text(b,24,y+12,name,15,active?C.accent:C.text,225,28,active?600:500);
    if(active)D.icon(b,'check',300,y+16,20,C.accent);
    if(occupied)D.text(b,272,y+13,'已占用',12,C.amber,66,26,400,false,'right');
    D.hit(b,12,y,name==='android-app'?316:366,52,active?null:occupied?'takeover':target,'Choose / '+name,active?'close':occupied?'overlay':'nav');
    if(name==='android-app'){
      D.icon(b,'more',342,y+16,22,C.muted);
      D.hit(b,328,y+2,48,48,'actions','Manage android-app','overlay');
    }
  }
  const y=96+items.length*56;
  D.rect(b,24,y+3,342,1,C.line,0,'New session divider');
  D.icon(b,'plus',25,y+22,22,C.accent);D.text(b,62,y+15,'新建会话',14,C.accent,260,34,500);
  D.hit(b,12,y+8,366,48,'new','New session','overlay');
}
D.boards.sessions.name='03 · 标题展开 / android-app';
picker(D.boards.sessions,'android-app','terminal',base);
D.boards.renamed.name='18 · 标题展开 / android-work';
picker(D.boards.renamed,'android-work','renamedTerminal',[['android-work','renamedTerminal'],...base.slice(1)]);
for(const [key,title,target,col,items] of [
  ['pickerDev','dev-server','devTerminal',0,base],
  ['pickerDeploy','deploy','deployTerminal',1,base],
  ['pickerNew','scratch','newTerminal',2,[...base,['scratch','newTerminal']]],
  ['pickerIdle','会话',null,3,base.slice(1)]
]){
  const b=D.box(null,col*480,160+8*960,390,326,C.bg,24,'标题展开 / '+title);
  D.boards[key]=b;picker(b,title,target,items);
}

// Remove the obsolete keyboard affordance and make the whole title tappable.
for(const key of ['terminal','keyboard','history','selection','extended','copied','recover','newTerminal','devTerminal','deployTerminal','renamedTerminal']){
  const b=D.boards[key],secondary=['newTerminal','devTerminal','deployTerminal','renamedTerminal'].includes(key);
  for(const s of [...b.children]){
    if(s.name==='Touch / Keyboard'||s.name==='Touch / Simulate interruption'||
      ((s.name==='Icon / keyboard'||s.name==='Icon / down')&&s.y-b.y<86))s.remove();
    if(s.name==='Touch / Session menu'&&secondary)unlink(s);
  }
  const title={newTerminal:'scratch',devTerminal:'dev-server',deployTerminal:'deploy',renamedTerminal:'android-work'}[key]||'android-app';
  const target={newTerminal:'pickerNew',devTerminal:'pickerDev',deployTerminal:'pickerDeploy',renamedTerminal:'renamed'}[key]||'sessions';
  arrow(b,title);D.hit(b,48,35,280,50,target,'Expand sessions','picker');
  if(key==='keyboard'){
    D.icon(b,'down',23,819,17,C.muted);D.hit(b,8,811,48,32,'terminal','System hide keyboard');
  }
}

// Closing a session leaves a terminal shell, not a separate management route.
let b=clear(D.boards.closed);b.name='19 · 终端 / 会话已结束';
header(b,'会话');D.gesture(b);
D.hit(b,48,35,280,50,'pickerIdle','Expand sessions','picker');
D.text(b,24,372,'会话已结束',15,C.muted,342,30,500,false,'center');
D.button(b,111,422,168,'选择会话','pickerIdle','secondary',null,'picker',44);

// The terminal menu is about the current session; switching lives in the title.
b=clear(D.boards.actions);b.resize(390,348);
D.rect(b,175,12,40,4,C.line,2,'Sheet handle');
D.text(b,24,29,'android-app',18,C.text,290,30,600);
D.icon(b,'close',338,34,24,C.muted);D.hit(b,326,22,48,48,null,'Close menu','close');
for(const [i,label,icon,target,kind] of [
  [0,'选择文本','copy','selection','nav'],[1,'历史','history','history','nav'],
  [2,'重命名','edit','rename','overlay'],[3,'断开','disconnect','home','nav'],
  [4,'结束会话','trash','close','overlay']
]){
  const y=81+i*48;D.icon(b,icon,26,y+9,22,i===4?C.red:C.muted);
  D.text(b,63,y+4,label,15,i===4?C.red:C.text,270,32,500);
  D.hit(b,12,y,366,48,target,label,kind);
}
const save=penpotUtils.findShape(s=>s.name==='Button / 保存名称',D.boards.rename);
unlink(save);D.link(save,'renamedTerminal');

// Wire title overlays at the top. Only the current-session row dismisses;
// selecting another row navigates to that exact illustrated terminal state.
for(const l of D.links){
  const dest=D.boards[l.target];
  if(l.kind!=='close'&&!dest)throw new Error('Missing target '+l.target);
  const action=l.kind==='close'?{type:'close-overlay'}:
    l.kind==='picker'?{type:'open-overlay',destination:dest,position:'top-center',closeWhenClickOutside:true,addBackgroundOverlay:false,animation:{type:'slide',way:'in',direction:'down',duration:180,easing:'ease-out'}}:
    l.kind==='overlay'?{type:'open-overlay',destination:dest,position:dest.width===390?'bottom-center':'center',closeWhenClickOutside:true,addBackgroundOverlay:true}:
    {type:'navigate-to',destination:dest};
  l.shape.addInteraction('click',action);
}
const guide=penpotUtils.findShape(s=>s.name==='Prototype guide',penpot.root);
const note=penpotUtils.findShape(s=>s.type==='text'&&s.characters.startsWith('①'),guide);
note.characters='① 首页：最近连接 / 设备 → 终端；齿轮 → 设置\n\n② 终端标题 → 展开会话；选择后切换；点外部收起\n\n③ 点底部输入区 → 键盘；系统底部箭头 → 收起\n\n④ 终端文字模拟滚动 / 长按；手柄模拟跨屏延伸；··· 保留操作\n\n⑤ 扫码：取景区 / 左下相册 / 右下票据';
for(const f of penpot.currentPage.flows.filter(f=>/^Flow \d+$/.test(f.name)))f.remove();
penpot.selection=[D.boards.terminal];penpot.viewport.zoomIntoView([D.boards.terminal,D.boards.sessions]);
return {revision:3,boards:Object.keys(D.boards).length,newLinks:D.links.length};
