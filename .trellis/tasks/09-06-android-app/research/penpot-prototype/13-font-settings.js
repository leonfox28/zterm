// v8a: font-size preview, preserving existing language/theme states.
const D=storage.zd,C=D.C;
if(penpot.currentPage.id!=='c828d3cf-7d4e-8145-8008-98f4fa037d49')throw new Error('Wrong page');
if(D.v8)throw new Error('v8a already applied');
D.v8={};const V=D.v8;
V.light={...C,bg:'#F5F8F3',panel:'#FFFFFF',panel2:'#E6EEE3',line:'#D4DED2',text:'#182219',muted:'#53634F',dim:'#71816B',accent:'#476A20'};
V.unlink=s=>{for(const i of [...s.interactions])i.remove();};
V.nav=(s,b)=>s.addInteraction('click',{type:'navigate-to',destination:b});
V.open=(s,b,pos='center')=>s.addInteraction('click',{type:'open-overlay',destination:b,position:pos,closeWhenClickOutside:true,addBackgroundOverlay:pos!=='top-center'});
V.hit=(p,x,y,w,h,name)=>{const s=D.rect(p,x,y,w,h,C.text,0,'Touch / '+name);s.fills=[{fillColor:C.text,fillOpacity:0.001}];return s;};
V.clone=(key,source,name,x,y)=>{const b=D.boards[source].clone();penpot.root.appendChild(b);b.name=name;b.x=x;b.y=y;D.boards[key]=b;return b;};
V.modal=(key,name,x,y,w,h,pal=C)=>{const b=D.box(null,x,y,w,h,pal.panel,24,name);b.showInViewMode=false;D.boards[key]=b;return b;};
V.close=(b,pal=C)=>{D.icon(b,'close',b.width-50,24,22,pal.muted);V.hit(b,b.width-62,12,48,48,'Close').addInteraction('click',{type:'close-overlay',destination:b});};
V.button=(b,x,y,w,label,color,fg)=>{const c=D.box(b,x,y,w,48,color,12,'Button / '+label);D.text(c,8,10,label,15,fg,w-16,28,500,false,'center');return c;};
const groups=[['zh','dark'],['zh','light'],['en','dark'],['en','light']];
const fonts={};
for(const [g,[lang,theme]] of groups.entries())for(const [i,size] of [12,14,16].entries()){
  const key='font_'+lang+'_'+theme+'_'+size,pal=theme==='light'?V.light:C;
  const b=V.modal(key,'字号 / '+lang+' / '+theme+' / '+size,i*410,18000+g*400,342,280,pal);
  fonts[key]=b;D.text(b,24,20,lang==='en'?'Terminal font size':'终端字号',19,pal.text,254,32,600);V.close(b,pal);
  D.rect(b,24,70,294,100,pal.bg,12,'Font preview');
  D.text(b,38,78,'$ pwd\n~/projects/zterm\n$',size,pal.text,266,84,400,true);
  for(const [j,value] of [12,14,16].entries()){
    const selected=value===size;
    const item=V.button(b,24+j*100,196,94,String(value),selected?pal.accent:pal.panel2,selected?pal.bg:pal.text);
    item.name='Font choice / '+value+(selected?' / selected':'');
    item.setPluginData('ztermFontChoice',String(value));
  }
}
for(const [key,b] of Object.entries(fonts))for(const s of b.children.filter(s=>s.name.startsWith('Font choice / '))){
  const target=fonts[key.replace(/\d+$/,s.getPluginData('ztermFontChoice'))];
  if(target.id!==b.id){s.addInteraction('click',{type:'close-overlay',destination:b});V.open(s,target);}
}
for(const b of Object.values(D.boards).filter(b=>b.getPluginData('ztermSettings'))){
  const pref=JSON.parse(b.getPluginData('ztermSettings')),en=pref.language==='en',light=pref.theme==='light',pal=light?V.light:C;
  for(const s of b.children){if(s.type==='text'&&['关于','About'].includes(s.characters))s.y=b.y+660;if(s.name==='About')s.y=b.y+696;}
  const row=D.box(b,16,578,358,56,pal.panel,16,'Preference / terminal font');
  D.text(row,18,14,en?'Terminal font size':'终端字号',15,pal.text,280,28,500);D.icon(row,'right',324,18,20,pal.muted);
  V.open(row,fonts['font_'+(en?'en':'zh')+'_'+(light?'light':'dark')+'_12']);
}
V.fonts=fonts;
for(const f of penpot.currentPage.flows.filter(f=>/^Flow \d+$/.test(f.name)))f.remove();
return {fontDialogs:Object.keys(fonts).length,settings:9};
