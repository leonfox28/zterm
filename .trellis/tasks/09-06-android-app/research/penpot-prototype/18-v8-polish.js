// v8f: align landscape expansion, inactive headers, and editor-only walkthrough.
const D=storage.zd,C=D.C,V=D.v8;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49'||!D.boards.ticketInvalid)throw new Error('Wrong page/order');
if(V.polished)throw new Error('v8f already applied');
const panel=D.boards.landscapeSessions;panel.resize(844,312);
for(const s of [...panel.children]){
  const y=s.y-panel.y;
  if(y<30&&s.x-panel.x>=300)s.x+=454;
  if(s.name==='Status indicator'&&y===17){s.remove();continue;}
  if(['Expanded app bar','App bar divider','Panel divider','Current session','New session divider','Touch / New session'].includes(s.name)||s.name.startsWith('Touch / Choose / '))s.resize(s.width+454,s.height);
  if(['Icon / check','Icon / more','已占用'].includes(s.name)||s.name.startsWith('Touch / Manage / '))s.x+=454;
  if(s.type==='text'&&['android-app','dev-server','deploy','新建会话'].includes(s.characters)&&y>=86)s.resize(s.width+454,s.height);
  if(s.name==='New session divider')s.y=panel.y+262;
  if(s.name==='Touch / New session')s.y=panel.y+264;
  if(s.name==='Icon / plus')s.y=panel.y+277;
  if(s.type==='text'&&s.characters==='新建会话')s.y=panel.y+270;
}
const landscape=D.boards.landscape;
for(const s of [...landscape.children]){
  if(s.name==='Status indicator'&&s.y-landscape.y===17){s.remove();continue;}
  if(s.type==='text'&&s.characters==='MacStudio'){s.x=landscape.x+61;s.y=landscape.y+59;}
  if(s.name==='Icon / down')s.x=landscape.x+139.7;
}
D.dot(landscape,50,68,4,C.green);
for(const key of ['firstOneConnecting','revoked'])for(const s of [...D.boards[key].children]){
  if(s.name==='Touch / Expand sessions')V.unlink(s);
  if(s.name==='Icon / down')s.remove();
}
const guide=penpotUtils.findShape(s=>s.name==='Prototype guide',penpot.root);
penpotUtils.findShape(s=>s.type==='text'&&s.characters.startsWith('①'),guide).characters=
  '① 首页直达终端；标题展开会话；每行 ··· 管理\n\n② 终端连续滚动、跨屏选择；八个快捷键常驻\n\n③ 扫码：系统相册 / 凭据弹窗；错误可修正重试\n\n④ 设置：语言、主题、字号预览；横屏增加列数\n\n⑤ 流程入口可查看长按移除、首次连接和异常状态';
V.polished=true;
penpot.selection=[D.boards.settings,landscape];penpot.viewport.zoomIntoView([landscape]);
return {polished:true};
