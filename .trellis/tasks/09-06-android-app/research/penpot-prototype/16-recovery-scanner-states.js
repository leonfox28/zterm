// v8d: interruption, revoked authorization, and scanner permission/image errors.
const D=storage.zd,C=D.C,V=D.v8;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49'||!D.boards.firstEntry)throw new Error('Wrong page/order');
if(D.boards.recoverFailed)throw new Error('v8d already applied');
const failed=V.clone('recoverFailed','recover','终端 / 重连失败',0,22000);
const waiting=D.boards.recover;
for(const s of failed.children){
  if(s.type==='text'&&s.characters==='连接中…'){s.characters='连接已中断';s.name=s.characters;}
  if(s.name==='Touch / Retry'){V.unlink(s);V.nav(s,waiting);}
}
for(const s of [...waiting.children]){
  if(['Disconnected content','Reconnect','Touch / Retry'].includes(s.name)||(s.type==='text'&&['连接中…','重试'].includes(s.characters)))s.remove();
  else if(s.type==='text'&&s.characters==='MacStudio'){s.characters='MacStudio · 重连中…';s.resize(230,21);}
}
waiting.addInteraction('after-delay',{type:'navigate-to',destination:D.boards.terminal},1200);
// Actual reconnect stays input-disabled until synchronized; delay is only a preview.
const revoked=V.clone('revoked','closed','终端 / 授权已失效',420,22000);
for(const s of revoked.children){
  if(s.type==='text'&&s.characters==='会话已结束'){s.characters='授权已失效';s.name=s.characters;}
  if(s.name==='Button / 选择会话'){s.name='Button / 重新配对';s.children.find(t=>t.type==='text').characters='重新配对';V.unlink(s);V.nav(s,D.boards.scanner);}
  if(s.name==='Touch / Expand sessions')V.unlink(s);
}
const denied=V.clone('scannerDenied','scanner','扫码 / 相机未授权',840,22000);
for(const s of [...denied.children])if(s.y-denied.y>=107&&s.y-denied.y<695)s.remove();
D.text(denied,24,336,'相机未授权',16,C.muted,342,32,400,false,'center');
V.nav(V.button(denied,111,390,168,'允许相机',C.accent,C.bg),D.boards.scanner);
const noQr=V.clone('imageNoQr','scanner','扫码 / 图片中无二维码',1260,22000);
D.rect(noQr,24,620,342,58,C.panel,12,'Image scan error');
D.text(noQr,40,635,'未识别到二维码',14,C.text,216,28,400);
D.text(noQr,278,635,'重选',14,C.accent,62,28,500,false,'center');
V.nav(V.hit(noQr,266,625,88,48,'Choose another system image'),D.boards.paired);
for(const [name,start] of [['连接 · 重连失败',failed],['连接 · 授权失效',revoked],['扫码 · 相机权限',denied],['扫码 · 图片无二维码',noQr]])penpot.currentPage.createFlow(name,start);
for(const f of penpot.currentPage.flows.filter(f=>/^Flow \d+$/.test(f.name)))f.remove();
return {failed:failed.id,revoked:revoked.id,denied:denied.id,noQr:noQr.id};
