const U=storage.su;if(Object.keys(U.boards).length)throw new Error('Already drawn');
U.screen('idle','01 · 设置底部 / 深色',0,0);
U.screen('checking','02 · 正在检查更新',1,0,false,'checking');
U.screen('latest','03 · 已是最新版 / Toast',2,0);
U.screen('available','04 · 发现新版本 / 深色',3,0);
U.screen('lightIdle','05 · 设置底部 / 浅色',0,1,true);
U.screen('lightAvailable','06 · 发现新版本 / 浅色',1,1,true);
U.screen('download','07 · 下载更新 / 深色',2,1);
U.screen('lightDownload','08 · 下载更新 / 浅色',3,1,true);
U.screen('latestStart','09 · 无更新路径 / 点击检查',0,2);
U.screen('latestChecking','10 · 无更新路径 / 检查中',1,2,false,'checking');
U.toast=(key,label)=>{const b=U.boards[key],C=U.refs[key].C;const toast=U.box(b,88,768,214,44,C.accent,22,'Toast / '+label);U.icon(toast,'check',16,13,18,C.onAccent);U.text(toast,44,10,label,14,C.onAccent,152,24,500);return toast;};
U.toast('latest','已是最新版');
U.modal=(key,downloading=false)=>{const b=U.boards[key],C=U.refs[key].C;const scrim=U.rect(b,0,0,390,844,'#000000',24,'Modal / Scrim');scrim.opacity=.54;
const m=U.box(b,24,258,342,324,C.panel,28,downloading?'Dialog / Download progress':'Dialog / Update available');
U.rect(m,24,24,44,44,C.panel2,14,'Dialog / Icon background');U.icon(m,'download',34,34,24,C.accent);
U.text(m,24,84,downloading?'正在下载更新':'发现新版本',22,C.text,294,34,600);
U.text(m,24,126,'0.1.34  →  0.1.35',14,C.accent,294,26,500,true);
if(downloading){U.text(m,24,171,'下载完成后将打开系统安装界面。',13,C.muted,294,24);U.rect(m,24,211,294,5,C.line,3,'Download / Track');U.rect(m,24,211,123,5,C.accent,3,'Download / Progress');U.text(m,24,222,'42%',12,C.muted,294,22,500,false,'right');U.refs[key].cancel=U.button(m,24,258,294,'取消下载',C);}
else{U.text(m,24,170,'是否下载并安装新版本？\n下载完成后将打开系统安装界面。',14,C.muted,294,54);U.refs[key].later=U.button(m,24,254,98,'稍后',C);U.refs[key].confirm=U.button(m,134,254,184,'下载并安装',C,true);}
U.refs[key].modal=m;return m;};
for(const key of ['available','lightAvailable'])U.modal(key);
for(const key of ['download','lightDownload'])U.modal(key,true);
U.go=(from,to,delay)=>from.addInteraction(delay===undefined?'click':'after-delay',{type:'navigate-to',destination:U.boards[to]},delay);
U.go(U.refs.idle.update,'checking');U.go(U.boards.checking,'available',850);U.go(U.refs.latestStart.update,'latestChecking');U.go(U.boards.latestChecking,'latest',850);U.go(U.boards.latest,'latestStart',2400);
U.go(U.refs.available.later,'idle');U.go(U.refs.available.confirm,'download');U.go(U.refs.download.cancel,'idle');
U.go(U.refs.lightIdle.update,'lightAvailable');U.go(U.refs.lightAvailable.later,'lightIdle');U.go(U.refs.lightAvailable.confirm,'lightDownload');U.go(U.refs.lightDownload.cancel,'lightIdle');
for(const [name,key] of [['01 · 发现更新：点击检查 → 确认下载','idle'],['02 · 无更新：点击检查 → Toast','latestStart'],['03 · 浅色主题','lightIdle'],['04 · 更新确认框','available']])penpot.currentPage.createFlow(name,U.boards[key]);
U.text(null,0,0,'设置 · 关于与应用更新',32,'#203529',1450,48,600);U.text(null,0,58,'设置页滚动到底部  /  延续现有配色与控件  /  Android · 390 × 844',16,'#53634F',1800,28);
const labels=[['idle','01 / 两个入口，放在设置最下方'],['checking','02 / 检查中，避免重复点击'],['latest','03 / 无更新：短暂 Toast'],['available','04 / 有更新：先询问，再下载'],['lightIdle','05 / 浅色主题'],['lightAvailable','06 / 浅色确认弹窗'],['download','07 / 确认后，展示下载进度'],['lightDownload','08 / 浅色下载状态'],['latestStart','09 / 原型分支：无更新'],['latestChecking','10 / 原型分支：检查中']];
for(const [key,label] of labels){const b=U.boards[key];U.text(null,b.x,b.y-42,label,15,'#203529',390,28,500);}
const note=U.box(null,940,2260,860,360,'#FFFFFF',20,'Review notes / Editor only');
U.text(note,28,24,'评审说明',24,'#203529',800,40,600);U.text(note,28,86,'• 新增入口位于现有通知设置之后；语言设置在上方滚动区域。\n• 版本 0.1.35、42% 为演示数据，不代表真实发布或下载。\n• GitHub 打开项目仓库；Toast 自动消失；“稍后”返回设置。\n• 确认仅演示下载状态，原型不会下载 APK 或触发系统安装。\n• 实际安装、权限与失败恢复在 GUI 确认后补充技术方案。',16,'#53634F',800,220);
penpot.selection=[U.boards.idle];penpot.viewport.zoomIntoView([U.boards.idle,U.boards.checking,U.boards.latest,U.boards.available]);
return {boards:Object.fromEntries(Object.entries(U.boards).map(([k,b])=>[k,{id:b.id,name:b.name}])),page:penpot.currentPage.id};
