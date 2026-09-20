const U=storage.su;if(U.notificationV2)throw new Error('Notification revision already applied');
U.paths.bell='M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9M10 21h4';
U.notification=(b,C,state='on')=>{
for(const s of [...b.children])if(s.type==='text'&&(s.characters==='终端通知'||s.characters==='系统通知设置'||s.characters.startsWith('连接有效时接收通知')))s.remove();
for(const s of [...b.children])if(s.name==='Notification / Preference row')s.remove();
if(!b.children.some(s=>s.type==='text'&&s.characters==='通知'))U.text(b,28,446,'通知',13,C.muted,330,22,500);
const row=U.box(b,16,478,358,74,C.panel,16,'Notification / Preference row');
U.text(row,16,11,'终端通知',15,C.text,248,26,500);
U.text(row,16,39,state==='blocked'?'系统未允许通知':state==='off'?'已关闭':'接收已连接终端的通知',12,C.muted,248,21);
const on=state==='on',sw=U.box(row,290,21,52,32,on?C.accent:C.panel2,16,'Switch / Terminal notifications / '+state);
if(!on)sw.strokes=[{strokeColor:C.muted,strokeWidth:1.5,strokeStyle:'solid'}];U.dot(sw,on?24:4,4,24,on?C.onAccent:C.muted);
return {row,switch:sw};};
for(const [key,b] of Object.entries(U.boards)){if(key.startsWith('connection'))continue;U.refs[key].notification=U.notification(b,U.refs[key].C);}
U.cloneState=(source,key,name,col,row,state)=>{const sourceBoard=U.boards[source],b=sourceBoard.clone();b.name=name;b.x=col*470;b.y=180+row*1040;U.boards[key]=b;U.refs[key]={C:U.refs[source].C};for(const s of penpotUtils.findShapes(s=>s.interactions?.length,b))for(const i of [...s.interactions])s.removeInteraction(i);U.refs[key].notification=U.notification(b,U.refs[key].C,state);return b;};
U.cloneState('idle','notificationsOff','15 · 终端通知 / 已关闭',0,5,'off');
U.cloneState('lightIdle','notificationsOffLight','16 · 终端通知 / 已关闭 / 浅色',1,5,'off');
U.cloneState('idle','notificationsBlocked','17 · 终端通知 / 系统未允许',2,5,'blocked');
U.cloneState('idle','notificationsPermission','18 · 开启系统通知 / 引导',3,5,'blocked');
U.go(U.refs.idle.notification.row,'notificationsOff');U.go(U.refs.notificationsOff.notification.row,'idle');
U.go(U.refs.lightIdle.notification.row,'notificationsOffLight');U.go(U.refs.notificationsOffLight.notification.row,'lightIdle');
U.go(U.refs.notificationsBlocked.notification.row,'notificationsPermission');
const b=U.boards.notificationsPermission,C=U.dark;
const scrim=U.rect(b,0,0,390,844,'#000000',24,'Modal / Scrim');scrim.opacity=.54;
const m=U.box(b,24,272,342,300,C.panel,28,'Dialog / System notification permission');
U.rect(m,24,24,44,44,C.panel2,14,'Dialog / Icon background');U.icon(m,'bell',34,34,24,C.accent);U.text(m,24,84,'开启系统通知',22,C.text,294,34,600);
U.text(m,24,134,'请在系统设置中允许 zterm 发送通知。\n返回后会自动更新开关状态。',14,C.muted,294,60);
const cancel=U.button(m,24,230,98,'取消',C),settings=U.button(m,134,230,184,'去设置',C,true);U.go(cancel,'notificationsBlocked');settings.name='Button / System settings handoff / Not simulated';
for(const button of [cancel,settings]){button.flex.wrap='nowrap';button.flex.alignContent='center';button.flex.horizontalPadding=8;button.flex.verticalPadding=11;}
for(const [key,label] of [['notificationsOff','15 / 应用内关闭：立即停止发送'],['notificationsOffLight','16 / 应用内关闭：浅色'],['notificationsBlocked','17 / 系统未允许：保持关闭'],['notificationsPermission','18 / 需要用户在系统设置中允许']]){const b=U.boards[key];U.text(null,b.x,b.y-42,label,15,'#203529',390,28,500);}
penpot.currentPage.createFlow('07 · 通知开关：开 → 关',U.boards.idle);
penpot.currentPage.createFlow('08 · 系统禁止通知：权限引导',U.boards.notificationsBlocked);
const note=U.box(null,940,2940,860,285,'#FFFFFF',20,'Review v2 / App notification switch');
U.text(note,28,20,'通知 · 应用内开关',23,'#203529',804,40,600);
U.text(note,28,78,'• 关闭：停止发送新通知；开启：检查系统权限。\n• 首次开启可申请 Android 系统权限，拒绝后保持关闭。\n• 系统已禁止时引导去设置；返回时读取实际授权状态。\n• 已存在的系统通知不因关闭开关而自动清空。\n• 原型开关、权限引导为独立示例，不更改手机系统设置。',15,'#53634F',804,168);
U.notificationV2=true;
await penpot.waitForLayoutUpdate();
return {newBoards:['notificationsOff','notificationsOffLight','notificationsBlocked','notificationsPermission'].map(k=>({key:k,id:U.boards[k].id})),page:penpot.currentPage.id};
