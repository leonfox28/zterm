// v8c: zero/one/multiple first-session routes inside the terminal container.
const D=storage.zd,C=D.C,V=D.v8;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49'||!D.boards.homeRemoved)throw new Error('Wrong page/order');
if(D.boards.firstEntry)throw new Error('v8c already applied');
const first=V.clone('firstEntry','closed','终端 / 首次选择会话',0,20900);
for(const s of [...first.children])if(s.type==='text'&&s.characters==='会话已结束'||s.name==='Button / 选择会话')s.remove();
const list=V.clone('firstSessions','sessions','首次连接 / 多个会话',420,20900);
for(const s of [...list.children]){
  if(s.name==='Current session'||s.name==='Icon / check')s.remove();
  else if(s.type==='text'&&s.y-list.y<86&&s.characters==='android-app'){s.characters='会话';s.name='会话';}
  else if(s.name==='Icon / up')s.x=list.x+123;
  else if(s.name==='Touch / Choose / android-app'){V.unlink(s);V.nav(s,D.boards.terminal);}
  else if(s.name==='Touch / Manage / android-app'){V.unlink(s);V.open(s,D.boards.rowActions0,'bottom-center');}
}
const expand=first.children.find(s=>s.name==='Touch / Expand sessions');V.unlink(expand);V.open(expand,list,'top-center');
first.addInteraction('after-delay',{type:'open-overlay',destination:list,position:'top-center',closeWhenClickOutside:true,addBackgroundOverlay:false},200);
const empty=V.clone('firstEmpty','closed','终端 / 暂无会话',840,20900);
const form=V.clone('newFromEmpty','new','新建会话 / 首个会话',1260,20900);
const sole=V.clone('firstCreated','newTerminal','终端 / 首个会话已创建',1680,20900);
const emptyList=V.clone('firstEmptyList','pickerIdle','标题展开 / 暂无会话',2100,20900);
for(const s of [...emptyList.children])if(s.y-emptyList.y>=86)s.remove();
emptyList.resize(390,154);D.rect(emptyList,0,86,390,68,C.panel,0,'Expanded app bar');
D.icon(emptyList,'plus',25,110,22,C.accent);D.text(emptyList,62,103,'新建会话',15,C.accent,260,34,500);
V.open(V.hit(emptyList,12,96,366,48,'New first session'),form,'bottom-center');
for(const s of empty.children){
  if(s.type==='text'&&s.characters==='会话已结束'){s.characters='暂无会话';s.name=s.characters;}
  if(s.name==='Button / 选择会话'){
    s.name='Button / 新建会话';s.children.find(t=>t.type==='text').characters='新建会话';V.unlink(s);V.open(s,form,'bottom-center');
  }
  if(s.name==='Touch / Expand sessions'){V.unlink(s);V.open(s,emptyList,'top-center');}
}
const soleList=V.clone('firstCreatedList','firstEmptyList','标题展开 / 首个 scratch',2520,20900);
soleList.resize(390,214);
for(const s of soleList.children){
  if(s.type==='text'&&s.characters==='会话'){s.characters='scratch';s.name='scratch';}
  if(s.name==='Expanded app bar')s.resize(390,128);
  else if(s.y-soleList.y>=96)s.y+=60;
}
D.rect(soleList,12,96,366,52,C.panel2,12,'Current session');D.text(soleList,24,108,'scratch',15,C.text,270,28,500);
D.icon(soleList,'check',304,112,20,C.accent);D.icon(soleList,'more',342,112,22,C.muted);
V.hit(soleList,12,96,316,52,'Current scratch').addInteraction('click',{type:'close-overlay',destination:soleList});
V.open(V.hit(soleList,328,98,48,48,'Manage scratch'),D.boards.rowActions3,'bottom-center');
const title=sole.children.find(s=>s.name==='Touch / Expand sessions');V.unlink(title);V.open(title,soleList,'top-center');
const create=form.children.find(s=>s.name==='Button / 创建并进入');V.unlink(create);V.nav(create,sole);
// Clone close actions are contextual; never close a different source overlay.
for(const modal of [form,emptyList,soleList])for(const s of penpotUtils.findShapes(s=>s.interactions?.some(i=>i.action.type==='close-overlay'),modal)){
  V.unlink(s);s.addInteraction('click',{type:'close-overlay',destination:modal});
}
const one=V.clone('firstOneConnecting','firstEntry','终端 / 首次连接单个会话',2940,20900);V.unlink(one);
D.text(one,24,368,'连接中…',16,C.muted,342,32,400,false,'center');
one.addInteraction('after-delay',{type:'navigate-to',destination:D.boards.devTerminal},900);
const freshCard=D.boards.homeNoRecent.children.find(s=>s.name==='Host / MacStudio');V.unlink(freshCard);V.nav(freshCard,first);
const enter=penpotUtils.findShape(s=>s.name==='Button / 进入终端',D.boards.paired);V.unlink(enter);V.nav(enter,first);
for(const [name,start] of [['首次连接 · 多个会话',first],['首次连接 · 无会话',empty],['首次连接 · 一个会话',one]])penpot.currentPage.createFlow(name,start);
for(const f of penpot.currentPage.flows.filter(f=>/^Flow \d+$/.test(f.name)))f.remove();
return {first:first.id,empty:empty.id,sole:one.id};
