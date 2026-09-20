const U=storage.su;if(U.boards.connectionFailure)throw new Error('Already drawn');
penpot.currentPage.name='02 · 设置更新与连接提示';
U.paths.disconnect='M3 3l18 18M3 8a15 15 0 0 1 3-2M10 4a15 15 0 0 1 11 4M6 12a10 10 0 0 1 3-1.6M14 10.7a10 10 0 0 1 4 1.3M9 16a5 5 0 0 1 6 0M12 20h.01';
U.paths.chevron='M6 9l6 6 6-6';
U.failure=(key,col,light=false,retry=false)=>{const C=light?U.light:U.dark,b=U.box(null,col*470,3460,390,844,C.bg,24,retry?'13 · 连接重试中':'连接失败 / '+(light?'浅色':'深色'));b.showInViewMode=true;U.boards[key]=b;U.refs[key]={C};
U.text(b,24,11,'9:41',13,C.text,46,22,600);U.icon(b,'wifi',307,14,17,C.text);U.rect(b,339,19,21,10,C.text,3,'Battery');U.rect(b,362,22,2,4,C.text,1);U.rect(b,133,830,124,4,C.muted,2,'System gesture handle');
U.icon(b,'back',20,55,22,C.text);U.text(b,60,44,'终端',19,C.text,150,28,600);U.icon(b,'chevron',105,50,18,C.muted);U.text(b,60,73,retry?'MacStudio · 正在连接…':'MacStudio · 无法连接',12,C.muted,260,22);U.rect(b,0,110,390,1,C.line,0,'App bar / Divider');
const card=U.box(b,24,292,342,284,C.panel,28,'Connection status / Consistent wide card');U.refs[key].card=card;
U.rect(card,24,24,44,44,C.panel2,14,'Status / Icon background');U.icon(card,retry?'spinner':'disconnect',34,34,24,C.accent);U.text(card,24,84,retry?'正在重新连接…':'连接失败',22,C.text,294,34,600);
U.text(card,24,134,retry?'正在尝试连接 MacStudio。\n请稍候。':'暂时无法连接到 MacStudio。\n请检查网络，或确认主机在线后重试。',14,C.muted,294,56);
if(!retry){U.refs[key].sessions=U.button(card,24,214,98,'会话',C);U.refs[key].retry=U.button(card,134,214,184,'重新连接',C,true);}
else{const pending=U.button(card,24,214,294,'连接中…',C);pending.opacity=.5;}
U.rect(b,0,756,390,1,C.line,0,'Toolbar / Divider');const toolbar=U.box(b,0,758,390,48,C.bg,0,'Terminal toolbar / Disabled');const f=toolbar.addFlexLayout();f.dir='row';f.columnGap=0;f.alignItems='center';['▧','＋','Esc','Tab','Ctrl','Alt','←','↓','↑','→'].forEach(label=>U.text(toolbar,0,0,label,12,C.muted,39,30,400,false,'center'));toolbar.opacity=.45;
U.text(null,b.x,b.y-42,retry?'13 / 重试中：保持同一宽度与位置':light?'12 / 连接失败：浅色':'11 / 连接失败：统一为宽卡片',15,'#203529',390,28,500);return b;};
U.failure('connectionFailure',0);U.failure('connectionFailureLight',1,true);U.failure('connectionRetry',2,false,true);U.failure('connectionRetryLight',3,true,true);
U.boards.connectionRetryLight.name='14 · 连接重试中 / 浅色';
U.go(U.refs.connectionFailure.retry,'connectionRetry');U.go(U.boards.connectionRetry,'connectionFailure',1600);U.go(U.refs.connectionFailureLight.retry,'connectionRetryLight');U.go(U.boards.connectionRetryLight,'connectionFailureLight',1600);
for(const [name,key] of [['05 · 连接失败提示 / 深色','connectionFailure'],['06 · 连接失败提示 / 浅色','connectionFailureLight']])penpot.currentPage.createFlow(name,U.boards[key]);
const note=U.box(null,0,4430,1800,150,'#FFFFFF',20,'Connection / Review notes');
U.text(note,28,20,'连接失败提示 · 本次一起调整',23,'#203529',1700,40,600);
U.text(note,28,72,'统一 342 px 宽度、28 px 圆角、24 px 内边距与 46 px 按钮。保持终端中的状态卡片语义与现有会话入口。\n演示为首次连接失败：不填充虚构终端输出。重试仅模拟等待后仍失败；会话切换及真实网络结果不在这版原型中模拟。',15,'#53634F',1720,55);
const heading=penpotUtils.findShape(s=>s.type==='text'&&s.characters==='设置 · 关于与应用更新',penpot.root);heading.characters='设置更新 & 连接状态';
for(const flow of [...penpot.currentPage.flows])if(!/^[0-9]{2} · /.test(flow.name))penpot.currentPage.removeFlow(flow);
penpot.selection=[U.boards.connectionFailure];penpot.viewport.zoomIntoView([U.boards.connectionFailure,U.boards.connectionFailureLight]);
return {page:penpot.currentPage.id,boards:Object.fromEntries(Object.entries(U.boards).map(([k,b])=>[k,{id:b.id,name:b.name}])),flows:penpot.currentPage.flows.map(f=>({name:f.name,start:f.startingBoard.id}))};
