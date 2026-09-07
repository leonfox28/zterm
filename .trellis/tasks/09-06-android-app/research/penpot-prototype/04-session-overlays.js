const D=storage.zd,C=D.C;
function closeIcon(p){D.icon(p,'close',p.width-47,28,22,C.muted);D.hit(p,p.width-59,17,48,48,null,'Close dialog','close');}
let b=D.overlay('new','13 · 新建会话',0,3,390,488);closeIcon(b);D.text(b,24,38,'新建会话',24,C.text,290,40,600);D.text(b,24,83,'在 MacStudio 上开始一段新工作。',13,C.muted,342,27);D.field(b,133,'会话名称','scratch');D.field(b,255,'工作目录（可选）','留空使用主机默认目录');D.button(b,24,402,342,'创建并进入','newTerminal','primary','plus');

b=D.overlay('actions','14 · 会话操作',1,3,390,458);closeIcon(b);D.text(b,24,34,'android-app',22,C.text,290,37,600);D.text(b,24,75,'MacStudio · 会话运行中',12,C.muted);
function action(y,label,icon,target,kind='nav',danger=false){D.icon(b,icon,27,y+12,21,danger?C.red:C.muted);D.text(b,66,y+7,label,15,danger?C.red:C.text,280,32,500);D.hit(b,14,y,362,49,target,label,kind);}
action(119,'浏览历史','history','history');action(174,'选择文本','copy','selection');action(229,'重命名','edit','rename','overlay');D.rect(b,24,285,342,1,C.line);action(297,'断开当前会话','disconnect','sessions');action(352,'结束会话…','trash','close','overlay',true);D.text(b,24,415,'断开连接不会结束主机上的工作。',11,C.dim,342,24);

b=D.overlay('rename','15 · 重命名会话',2,3,390,352);closeIcon(b);D.text(b,24,38,'重命名会话',24,C.text,290,40,600);D.field(b,112,'新名称','android-work');D.text(b,24,213,'修改名称不会重启会话或中断工作。',12,C.muted,342,28);D.button(b,24,272,342,'保存名称','renamed','primary','check');

b=D.overlay('close','16 · 结束会话确认',3,3,342,312);D.icon(b,'trash',24,26,27,C.red);D.text(b,24,78,'结束这个会话？',22,C.text,294,36,600);D.text(b,24,126,'android-app',14,C.text,294,25,600,true);D.text(b,24,162,'这会终止主机上的会话和正在运行的进程。\n此操作无法恢复。',13,C.muted,294,58);D.button(b,24,237,141,'取消',null,'secondary',null,'close');D.link(b.children[b.children.length-1],null,'close');D.button(b,177,237,141,'结束会话','closed','danger');

b=D.overlay('takeover','21 · 接管会话确认',0,5,342,354);D.icon(b,'lock',24,27,28,C.amber);D.text(b,24,79,'会话正在电脑上使用',21,C.text,294,37,600);D.text(b,24,130,'deploy · MacStudio',13,C.text,294,25,500,true);D.text(b,24,171,'接管后，电脑将失去控制权。\n会话中的程序会继续运行。',13,C.muted,294,62);D.button(b,24,277,141,'取消',null,'secondary');D.link(b.children[b.children.length-1],null,'close');D.button(b,177,277,141,'接管会话','deployTerminal','primary');

b=D.overlay('hostOffline','22 · 主机暂不可达',1,5,342,304);D.icon(b,'wifi',24,27,28,C.amber);D.text(b,24,78,'暂时无法连接主机',22,C.text,294,36,600);D.text(b,24,123,'build-server',14,C.text,294,25,500,true);D.text(b,24,163,'请确认主机在线，网络连接正常，\n然后再次尝试。',13,C.muted,294,57);D.button(b,24,239,294,'返回主机',null,'secondary');D.link(b.children[b.children.length-1],null,'close');
const buildLink=D.links.find(l=>l.shape.name==='Host / build-server');buildLink.target='hostOffline';buildLink.kind='overlay';

function sessionResult(key,name,col,row,renamed=false){const p=D.screen(key,name,col,row);D.nav(p,'MacStudio','已连接 · 连接状态良好');D.text(p,24,148,'会话',30,C.text,200,46,600);D.text(p,24,198,'切换会话，工作会继续运行。',14,C.muted);D.tag(p,289,157,renamed?'3 个会话':'2 个会话',77);const rows=renamed?[['android-work','~/projects/zterm','当前手机','renamedTerminal'],['dev-server','~/projects/zterm','空闲','devTerminal'],['deploy','~/deploy','电脑正在使用','takeover']]:[['dev-server','~/projects/zterm','空闲','devTerminal'],['deploy','~/deploy','电脑正在使用','takeover']];rows.forEach(([label,path,tag,target],i)=>{const s=D.box(p,24,254+i*140,342,124,C.panel,18,'Session / '+label);D.icon(s,'terminal',18,21,23,i===0&&renamed?C.accent:C.muted);D.text(s,54,16,label,18,C.text,242,30,600);D.text(s,20,59,path,12,C.muted,302,23,400,true);D.tag(s,20,90,tag,92,i===0&&renamed);D.icon(s,'right',304,24,18,C.muted);D.link(s,target,target==='takeover'?'overlay':'nav');});D.rect(p,78,680,234,44,'#314A30',12,'Result message');D.icon(p,'check',92,692,18,C.accent);D.text(p,121,689,renamed?'会话已重命名':'会话已结束',13,C.text,183,27,500);D.button(p,24,748,342,'新建会话','new','primary','plus','overlay');return p;}
sessionResult('renamed','18 · 重命名完成',1,4,true);sessionResult('closed','19 · 会话已结束',2,4,false);

function secondaryTerminal(key,name,col,row,session,lines){const p=D.termBase(key,name,col,row,'live',session);D.output(p,lines);D.liveFooter(p);return p;}
secondaryTerminal('newTerminal','20 · 新会话已创建',3,4,'scratch',[['$ pwd','text'],['/Users/leon','blue'],['','muted'],['$','text']]);
secondaryTerminal('devTerminal','23 · 切换到开发服务',2,5,'dev-server',[['$ npm run dev','text'],['','muted'],['  VITE  ready in 284 ms','green'],['','muted'],['  Local: http://localhost:5173/','blue'],['','muted'],['  press h + enter to show help','muted']]);
secondaryTerminal('deployTerminal','24 · 接管部署会话',3,5,'deploy',[['$ ./deploy.sh --status','text'],['','muted'],['Checking services...','muted'],['web       running','green'],['worker    running','green'],['database  healthy','green'],['','muted'],['All services are available.','text'],['','muted'],['$','text']]);
secondaryTerminal('renamedTerminal','25 · 重命名后继续工作',0,6,'android-work',D.liveLines);
// Secondary terminal demos return to their owning list; the detailed actions
// flow is demonstrated on android-app so it never targets the wrong session.
for(const k of ['newTerminal','devTerminal','deployTerminal','renamedTerminal']){
const p=D.boards[k];for(const l of D.links.filter(l=>l.shape.parent?.id===p.id)){
 if(l.target==='actions'){l.target=k==='renamedTerminal'?'renamed':'sessions';l.kind='nav';}
 if(l.target==='keyboard'||l.target==='history'){l.target=k==='renamedTerminal'?'renamed':'sessions';l.kind='nav';}
}
}
return {boards:Object.keys(D.boards),pendingLinks:D.links.length};
