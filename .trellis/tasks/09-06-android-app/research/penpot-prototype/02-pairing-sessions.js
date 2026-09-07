const D=storage.zd,C=D.C;
let b=D.screen('home','01 · 主机',0,0);
D.icon(b,'terminal',25,67,27,C.accent);D.text(b,64,64,'zterm',22,C.text,120,32,600);D.tag(b,289,69,'ANDROID',77);
D.text(b,24,136,'你的主机',32,C.text,300,48,600);D.text(b,24,190,'随时回到正在进行的工作。',14,C.muted);
const hero=D.box(b,24,244,342,216,C.panel,20,'Continue session');
D.dot(hero,20,23,7,C.accent);D.text(hero,36,14,'继续上次的会话',12,C.accent,265,26,500);
D.text(hero,20,56,'android-app',25,C.text,302,38,600);D.text(hero,20,101,'MacStudio  ·  ~/projects/zterm',12,C.muted,302,24,400,true);
D.button(hero,20,148,302,'继续会话','terminal','primary','terminal');
D.text(b,24,496,'已保存的主机',13,C.muted,220,24,500);D.text(b,336,496,'2',13,C.dim,30,24,500,false,'right');
let card=D.box(b,24,536,342,78,C.panel,16,'Host / MacStudio');D.icon(card,'server',16,24,28,C.accent);D.text(card,60,14,'MacStudio',16,C.text,210,25,600);D.text(card,60,42,'已连接 · 3 个会话',12,C.muted,210,22);D.icon(card,'right',302,27,20,C.muted);D.link(card,'sessions');
card=D.box(b,24,626,342,78,C.panel,16,'Host / build-server');D.icon(card,'server',16,24,28,C.muted);D.text(card,60,14,'build-server',16,C.text,210,25,600);D.text(card,60,42,'未连接 · 上次使用于昨天',12,C.muted,245,22);D.icon(card,'right',302,27,20,C.muted);D.link(card,'recover');
D.button(b,24,748,342,'添加主机','scanner','secondary','plus');

b=D.screen('scanner','02 · 扫码配对',1,0,'#0A100C');D.nav(b,'添加主机','扫描主机的配对二维码');
D.rect(b,28,179,334,348,'#121C16',24,'Camera preview');
D.rect(b,55,224,280,228,'#1B2920',12,'Host screen in camera');D.rect(b,68,237,254,17,'#283B2C',4);D.dot(b,79,243,5,C.dim);D.dot(b,91,243,5,C.dim);D.dot(b,103,243,5,C.dim);
D.qr(b,116,267,158,'#E0E9DC','#111B13');D.text(b,72,468,'将二维码放入框内',12,C.muted,246,24,400,false,'center');
for(const [x,y,dx,dy] of [[59,210,1,1],[329,210,-1,1],[59,498,1,-1],[329,498,-1,-1]]){D.rect(b,dx===1?x:x-26,y,28,3,C.accent,1);D.rect(b,x,dy===1?y:y-26,3,28,C.accent,1);}
D.rect(b,68,358,254,1,C.accent,0,'Scan line');D.hit(b,58,209,273,291,'paired','Simulate scan detection');
D.text(b,36,565,'把电脑上的工作，带在身边',19,C.text,318,30,600,false,'center');D.text(b,36,611,'在主机中打开配对二维码，\n对准后将自动识别。',14,C.muted,318,50,400,false,'center');
D.icon(b,'album',43,739,26,C.accent);D.text(b,79,738,'相册',15,C.text,76,28,500);D.hit(b,24,724,135,60,'album','Choose QR image');
D.icon(b,'ticket',235,739,26,C.accent);D.text(b,271,738,'填入票据',15,C.text,95,28,500);D.hit(b,216,724,150,60,'ticket','Enter pairing ticket');

b=D.screen('sessions','03 · 会话',2,0);D.nav(b,'MacStudio','已连接 · 连接状态良好');
D.text(b,24,148,'会话',30,C.text,200,46,600);D.text(b,24,198,'切换会话，工作会继续运行。',14,C.muted);D.tag(b,289,157,'3 个会话',77);
function session(y,name,desc,tag,active,target,menu=false){const s=D.box(b,24,y,342,124,C.panel,18,'Session / '+name);if(active)D.rect(s,0,22,3,80,C.accent,1);D.icon(s,'terminal',18,21,23,active?C.accent:C.muted);D.text(s,54,16,name,18,C.text,222,30,600);D.text(s,20,59,desc,12,C.muted,302,23,400,true);D.tag(s,20,90,tag,active?76:88,active);D.hit(s,0,0,286,124,target,'Open '+name,target==='takeover'?'overlay':'nav');if(menu){D.icon(s,'more',300,21,23,C.muted);D.hit(s,288,8,48,48,'actions','Session menu','overlay');}else{D.icon(s,'right',303,23,18,C.muted);D.hit(s,290,0,52,124,target,'Open '+name,target==='takeover'?'overlay':'nav');}return s;}
session(254,'android-app','~/projects/zterm','当前手机',true,'terminal',true);
session(394,'dev-server','~/projects/zterm','空闲',false,'devTerminal');
session(534,'deploy','~/deploy','电脑正在使用',false,'takeover');
D.button(b,24,748,342,'新建会话','new','primary','plus','overlay');

b=D.screen('album','05 · 相册选择',0,1);D.nav(b,'选择二维码图片','系统照片选择器', 'scanner');
D.text(b,24,146,'照片',28,C.text,260,42,600);D.tag(b,290,154,'相册',76);D.text(b,24,209,'今天',13,C.muted);
const photos=[[24,251],[142,251],[260,251],[24,369],[142,369],[260,369]];
for(let i=0;i<photos.length;i++){const [x,y]=photos[i];D.rect(b,x,y,106,106,i===0?'#E1E9DB':i%2?'#2C4032':'#253A34',12,'Photo thumbnail '+(i+1));if(i===0){D.qr(b,x+12,y+12,82,'#172117','#E1E9DB');D.hit(b,x,y,106,106,'paired','Select QR image');}else{D.rect(b,x+10,y+58,86,36,i%2?'#43654C':'#56706A',5);D.dot(b,x+64,y+18,16,'#6E8965');}}
D.text(b,24,532,'只会读取你选择的图片。',13,C.muted,342,28);D.button(b,24,748,342,'取消','scanner','secondary');

b=D.screen('ticket','06 · 填入票据',1,1);D.nav(b,'填入票据','无需使用相机', 'scanner');
D.text(b,24,156,'粘贴主机的配对票据',24,C.text,342,38,600);D.text(b,24,211,'从主机复制完整票据，确认后即可连接。',14,C.muted,342,46);
D.field(b,289,'配对票据','zterm-pair://\n•••• •••• •••• ••••\n•••• •••• •••• ••••',null,3);
D.icon(b,'lock',25,490,18,C.dim);D.text(b,53,486,'票据仅用于本次配对，请勿转发。',12,C.muted,313,30);
D.button(b,24,586,342,'粘贴票据',null,'secondary','copy');D.button(b,24,660,342,'确认连接','paired','primary','right');
D.text(b,24,740,'无法连接？可返回扫描新的二维码。',12,C.dim,342,32,400,false,'center');

b=D.screen('paired','07 · 配对成功',2,1);D.nav(b,'连接主机','', 'scanner');
D.dot(b,149,198,92,'#243922');D.icon(b,'check',171,220,48,C.accent);D.text(b,24,324,'主机已添加',29,C.text,342,46,600,false,'center');D.text(b,24,385,'MacStudio',20,C.text,342,30,600,false,'center');D.text(b,40,432,'以后可以直接从主机列表连接。',14,C.muted,310,48,400,false,'center');
D.rect(b,24,535,342,90,C.panel,18);D.icon(b,'server',43,566,28,C.accent);D.text(b,87,551,'MacStudio',16,C.text,240,30,600);D.text(b,87,582,'3 个会话可用',12,C.muted,240,23);
D.button(b,24,715,342,'查看会话','sessions','primary','right');
return {created:['home','scanner','sessions','album','ticket','paired'].map(k=>({key:k,id:D.boards[k].id}))};
