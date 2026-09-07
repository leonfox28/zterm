// v4 user review: no terminal overflow; every Session row has Rename / Delete.
// Planning prototype only. Secondary mutation confirmations remain visual endpoints.
const D=storage.zd,C=D.C;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49')throw new Error('Wrong page');
if(D.boards.rowActions0)throw new Error('v4 already applied');
D.links=[];
function clear(b){for(const s of [...b.children])s.remove();return b;}
function unlink(s){for(const i of [...s.interactions])i.remove();}
function closeIcon(b){D.icon(b,'close',338,34,24,C.muted);D.hit(b,326,22,48,48,null,'Dismiss menu','close');}
const names=['android-app','dev-server','deploy','scratch','android-work'];
const panelKeys=['sessions','renamed','pickerDev','pickerDeploy','pickerNew','pickerIdle'];
const menuByName={};
for(const [i,name] of names.entries()){
  const key='rowActions'+i,b=D.overlay(key,'会话操作 / '+name,i%4,9+Math.floor(i/4),390,202);
  menuByName[name]=key;D.text(b,24,29,name,18,C.text,290,30,600);closeIcon(b);
  // Each name gets its own form/confirmation so a row never points at another Session.
  // The original android-app mutation walkthrough stays wired only in its current panel.
  for(const [type,source] of [['rowRename','rename'],['rowClose','close']]){
    const clone=D.boards[source].clone();penpot.root.appendChild(clone);
    clone.name=(type==='rowRename'?'重命名 / ':'结束确认 / ')+name;
    clone.x=(i%4)*480;clone.y=160+(11+Math.floor(i/4)*2+(type==='rowClose'?1:0))*960;
    clone.showInViewMode=false;D.boards[type+i]=clone;
    for(const s of penpotUtils.findShapes(s=>s.interactions?.length,clone)){
      const dismiss=s.interactions.some(a=>a.action.type==='close-overlay');
      unlink(s);if(dismiss)s.addInteraction('click',{type:'close-overlay'});
    }
    for(const s of penpotUtils.findShapes(s=>s.type==='text',clone)){
      if(s.characters==='android-app'||s.characters==='android-work'){s.characters=name;s.name=name;}
    }
    if(type==='rowRename'){
      const save=penpotUtils.findShape(s=>s.name==='Button / 保存名称',clone);
      save.opacity=0.35;save.name='Button / 保存（名称未改）';
    }
  }
  for(const [j,label,icon,target] of [[0,'重命名','edit','rowRename'+i],[1,'删除','trash','rowClose'+i]]){
    const y=81+j*48;D.icon(b,icon,26,y+9,22,j?C.red:C.muted);
    D.text(b,63,y+4,label,15,j?C.red:C.text,270,32,500);
    D.hit(b,12,y,366,48,target,label,'overlay');
  }
}
// Preserve the demonstrated mutation flow on its exact current-session context.
const currentMenu=D.boards.rowActions0.clone();penpot.root.appendChild(currentMenu);
currentMenu.name='会话操作 / android-app 当前';currentMenu.x=2*480;currentMenu.y=160+10*960;
D.boards.rowActionsCurrent=currentMenu;
for(const s of penpotUtils.findShapes(s=>s.interactions?.length,currentMenu))unlink(s);
for(const s of currentMenu.children){
  if(s.name==='Touch / Dismiss menu')D.link(s,null,'close');
  if(s.name==='Touch / 重命名')D.link(s,'rename','overlay');
  if(s.name==='Touch / 删除')D.link(s,'close','overlay');
}

for(const key of panelKeys){
  const b=D.boards[key];
  for(const s of [...b.children])if(s.name==='Icon / more'||s.name.startsWith('Touch / Manage '))s.remove();
  for(const s of b.children.filter(s=>s.name==='已占用')){s.x=b.x+250;s.resize(66,26);}
  const rows=b.children.filter(s=>s.name.startsWith('Touch / Choose / '));
  for(const row of rows){
    const name=row.name.slice('Touch / Choose / '.length),y=row.y-b.y;
    row.resize(316,52);
    D.icon(b,'more',342,y+16,22,C.muted);
    const target=key==='sessions'&&name==='android-app'?'rowActionsCurrent':menuByName[name];
    D.hit(b,328,y+2,48,48,target,'Manage / '+name,'overlay');
  }
}

// Scroll/selection are direct gestures; Back leaves the terminal. No tools menu.
for(const key of ['terminal','keyboard','history','selection','extended','copied','recover','newTerminal','devTerminal','deployTerminal','renamedTerminal']){
  const b=D.boards[key];
  for(const s of [...b.children])if(s.name==='Touch / Session menu'||(s.name==='Icon / more'&&s.y-b.y<86))s.remove();
}
D.boards.actions.remove();delete D.boards.actions;
for(const b of Object.values(D.boards))for(const s of penpotUtils.findShapes(s=>s.type==='text',b)){
  if(s.characters==='结束会话？'){s.characters='删除会话？';s.name=s.characters;}
  if(s.characters==='结束会话'){s.characters='删除';s.name=s.characters;}
  if(s.characters==='将终止会话及其中的进程。'){s.characters='将结束会话及其中的进程。';s.name=s.characters;}
}
for(const l of D.links){
  const dest=D.boards[l.target];
  l.shape.addInteraction('click',l.kind==='close'?{type:'close-overlay'}:
    l.kind==='overlay'?{type:'open-overlay',destination:dest,position:dest.width===390?'bottom-center':'center',closeWhenClickOutside:true,addBackgroundOverlay:true}:
    {type:'navigate-to',destination:dest});
}
// Shared menu clones were made before the pending links were attached.
// Their three click targets were deliberately queued separately above.
const guide=penpotUtils.findShape(s=>s.name==='Prototype guide',penpot.root);
const note=penpotUtils.findShape(s=>s.type==='text'&&s.characters.startsWith('①'),guide);
note.characters='① 首页：最近连接 / 设备 → 终端；齿轮 → 设置\n\n② 终端标题 → 会话列表；每行 ··· → 重命名 / 删除\n\n③ 点底部输入区 → 键盘；系统底部箭头 → 收起\n\n④ 终端文字模拟滚动 / 长按；手柄模拟跨屏延伸\n\n⑤ 扫码：取景区 / 左下相册 / 右下票据';
for(const f of penpot.currentPage.flows.filter(f=>/^Flow \d+$/.test(f.name)))f.remove();
penpot.selection=[D.boards.sessions];penpot.viewport.zoomIntoView([D.boards.sessions,D.boards.rowActionsCurrent]);
return {revision:4,boards:Object.keys(D.boards).length,rows:panelKeys.reduce((n,k)=>n+D.boards[k].children.filter(s=>s.name.startsWith('Touch / Manage / ')).length,0)};
