// v6 planning prototype: credential dialog and independent language/theme choices.
const D=storage.zd,C=D.C;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49')throw new Error('Wrong page');
if(D.boards.settings_en_light)throw new Error('v6 already applied');
D.links=[];
function clear(b){for(const s of [...b.children])s.remove();return b;}
function unlink(s){for(const i of [...s.interactions])i.remove();}
function nav(s,b){s.addInteraction('click',{type:'navigate-to',destination:b});}
function open(s,b){s.addInteraction('click',{type:'open-overlay',destination:b,position:'center',closeWhenClickOutside:true,addBackgroundOverlay:true});}
function hit(b,x,y,w,h,name){const s=D.rect(b,x,y,w,h,C.text,0,'Touch / '+name);s.fills=[{fillColor:C.text,fillOpacity:0.001}];return s;}

// Reuse stable IDs; these are modal states, never standalone routes/screens.
for(const [key,filled] of [['ticket',false],['ticketReady',true]]){
  const b=clear(D.boards[key]);b.resize(342,300);b.showInViewMode=false;
  b.name=filled?'凭据弹窗 / 已填入':'凭据弹窗 / 空白';
  b.fills=[{fillColor:C.panel,fillOpacity:1}];b.borderRadius=24;
  D.text(b,24,20,'连接凭据',20,C.text,240,32,600);
  D.icon(b,'close',292,24,22,C.muted);
  const dismiss=hit(b,280,12,48,48,'Dismiss credentials');
  dismiss.addInteraction('click',{type:'close-overlay',destination:b});
  D.rect(b,24,72,294,124,C.bg,12,'Credential field');
  D.text(b,38,86,filled?'zterm-pair://\n•••• •••• •••• ••••':'输入或粘贴凭据',14,filled?C.text:C.dim,266,88,400,filled);
  D.text(b,24,218,'粘贴',15,C.accent,72,48,500,false,'center');
  const paste=hit(b,24,218,72,48,filled?'Paste credentials again':'Simulate paste credentials');
  const confirm=D.button(b,210,218,108,'连接',null,'primary',null,'nav',48);
  if(filled){
    nav(confirm,D.boards.paired);
  }else{
    confirm.opacity=0.35;
    const field=hit(b,24,72,294,124,'Simulate credential input');
    // Penpot has no editable inputs. Two ordered click actions replace the
    // empty overlay with its filled illustration without navigating the base.
    for(const s of [paste,field]){
      s.addInteraction('click',{type:'close-overlay',destination:b});
      open(s,D.boards.ticketReady);
    }
  }
}
const ticketHit=penpotUtils.findShape(s=>s.name==='Touch / Ticket',D.boards.scanner);
unlink(ticketHit);open(ticketHit,D.boards.ticket);
for(const s of D.boards.scanner.children.filter(s=>s.type==='text'&&['票据','填入票据'].includes(s.characters))){s.characters='填入凭据';s.name='填入凭据';}

// Nine states model both independent preferences in a static prototype.
// System settings in this illustration resolve to Chinese + dark.
const languages=['system','zh','en'],themes=['system','dark','light'];
const states={};let n=0;
for(const lang of languages)for(const theme of themes){
  const key='settings_'+lang+'_'+theme;
  let b;
  if(lang==='system'&&theme==='system')b=clear(D.boards.settings);
  else {b=D.screen(key,'设置 / '+lang+' / '+theme,4+n%3,15+Math.floor(n/3));clear(b);n++;}
  b.name='设置 / '+lang+' / '+theme;b.showInViewMode=true;
  b.setPluginData('ztermSettings',JSON.stringify({language:lang,theme}));
  states[lang+'_'+theme]=b;
}
const light={bg:'#F5F8F3',panel:'#FFFFFF',panel2:'#E6EEE3',line:'#D4DED2',text:'#182219',muted:'#53634F',dim:'#71816B',accent:'#476A20'};
function recolorLight(b){
  const map=Object.fromEntries(Object.entries(light).map(([k,v])=>[C[k].toLowerCase(),v]));
  for(const s of [b,...penpotUtils.findShapes(()=>true,b)]){
    if(s.fills?.length)s.fills=s.fills.map(f=>f.fillColor&&map[f.fillColor.toLowerCase()]?{...f,fillColor:map[f.fillColor.toLowerCase()]}:f);
    if(s.strokes?.length)s.strokes=s.strokes.map(v=>v.strokeColor&&map[v.strokeColor.toLowerCase()]?{...v,strokeColor:map[v.strokeColor.toLowerCase()]}:v);
  }
}
for(const lang of languages)for(const theme of themes){
  const b=states[lang+'_'+theme],en=lang==='en';
  D.status(b);D.gesture(b);D.icon(b,'back',16,49,22,C.text);
  nav(hit(b,4,37,48,48,'Back'),D.boards.home);
  D.text(b,58,45,en?'Settings':'设置',19,C.text,270,30,600);
  function group(y,title,values,labels,current,kind){
    D.text(b,24,y,title,13,C.muted,342,26,500);
    const card=D.box(b,16,y+36,358,168,C.panel,16,'Preference / '+kind);
    values.forEach((value,i)=>{
      const yy=i*56,selected=current===value;
      if(i)D.rect(card,58,yy,284,1,C.line,0,'Row divider');
      const ring=D.dot(card,22,yy+18,20,C.panel);ring.name='Radio / '+value+(selected?' / selected':'');
      ring.strokes=[{strokeColor:selected?C.accent:C.dim,strokeWidth:1.6,strokeStyle:'solid'}];
      if(selected)D.dot(card,27,yy+23,10,C.accent);
      D.text(card,58,yy+14,labels[i],15,C.text,276,28,selected?500:400);
      const target=kind==='language'?states[value+'_'+theme]:states[lang+'_'+value];
      const touch=hit(card,0,yy,358,56,'Set '+kind+' / '+value);
      if(!selected)nav(touch,target);
    });
  }
  group(110,en?'Language':'语言',languages,en?['System default','中文','English']:['跟随系统','中文','英文'],lang,'language');
  group(346,en?'Theme':'主题',themes,en?['System default','Dark','Light']:['跟随系统','深色','浅色'],theme,'theme');
  D.text(b,24,590,en?'About':'关于',13,C.muted,342,26,500);
  const about=D.box(b,16,626,358,66,C.panel,16,'About');
  D.text(about,18,18,'zterm',16,C.text,120,30,600);
  D.text(about,142,20,en?'Development build':'开发版',12,C.muted,198,26,400,false,'right');
  if(theme==='light')recolorLight(b);
}
const guide=penpotUtils.findShape(s=>s.name==='Prototype guide',penpot.root);
const note=penpotUtils.findShape(s=>s.type==='text'&&s.characters.startsWith('①'),guide);
note.characters='① 首页 → 终端；标题 → 会话列表；每行 ··· → 重命名 / 删除\n\n② 同一个终端：文字点击模拟滚动 / 长按；快捷键常驻\n\n③ 扫码右下角 → 凭据弹窗；输入 / 粘贴模拟填入后连接\n\n④ 设置可切换语言与主题；本稿系统值示例为中文、深色\n\n⑤ 相册交给 Android 系统；点击模拟返回图片后的配对结果';
for(const f of penpot.currentPage.flows.filter(f=>/^Flow \d+$/.test(f.name)))f.remove();
penpot.selection=[D.boards.settings];penpot.viewport.zoomIntoView([D.boards.settings,D.boards.ticket]);
return {revision:6,settings:Object.fromEntries(Object.entries(states).map(([k,b])=>[k,b.id])),ticket:D.boards.ticket.id,ticketReady:D.boards.ticketReady.id};
