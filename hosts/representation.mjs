// Transaction and transport host for compiler-sealed representation executors.
// These identifiers and carriers come from checked witnesses, not a live database.
function __repConstruct(datatype,constructor,arguments_=[]){return {datatype,constructor,arguments:arguments_};}
function __repData(v,id){if(!v||v.datatype!==id||!Number.isInteger(v.constructor)||!Array.isArray(v.arguments))throw Error('representation carrier mismatch');return [v.constructor,v.arguments];}
function __repEqual(a,b){const pending=[[a,b]];while(pending.length){const[x,y]=pending.pop();if(x===y)continue;if(!x||!y||typeof x!=='object'||typeof y!=='object'||x.datatype!==y.datatype||x.constructor!==y.constructor||x.arguments.length!==y.arguments.length)return false;for(let i=0;i<x.arguments.length;i++)pending.push([x.arguments[i],y.arguments[i]]);}return true;}
function __repTypeId(c,t){const s=c.types[JSON.stringify(t)];if(!s?.Data)throw Error('source datatype not projected');return s.Data;}
function __repEncode(c,t,v,depth=0){
 if(depth>32)throw Error('representation source depth');
 if(t==='U64'){if(typeof v!=='bigint'||v<0n||v>18446744073709551615n)throw Error('representation U64');return v;}
 if(t==='Bool'){if(typeof v!=='boolean')throw Error('representation Bool');return v;}
 const id=__repTypeId(c,t),inner=(t,v)=>__repEncode(c,t,v,depth+1);
 if(t==='Unit'){if(v!==null)throw Error('representation Unit');return __repConstruct(id,0);}
 if(t.List){if(!Array.isArray(v)||v.length>4096)throw Error('representation list limit');let tail=__repConstruct(id,0);for(let i=v.length-1;i>=0;i--)tail=__repConstruct(id,1,[inner(t.List,v[i]),tail]);return tail;}
 if(t.Named){const name=t.Named;if(c.enums[name]){const i=c.enums[name].findIndex(x=>v===name+'.'+x);if(i<0)throw Error('representation enum');return __repConstruct(id,i);}const fields=c.records[name];if(!v||typeof v!=='object'||Object.keys(v).length!==fields.length)throw Error('representation record');return __repConstruct(id,0,fields.map(([n,t])=>inner(t,v[n])));}
 if(t.Option){if(v&&Object.keys(v).length===1&&Object.hasOwn(v,'Some'))return __repConstruct(id,1,[inner(t.Option,v.Some)]);if(v?.None===null&&Object.keys(v).length===1)return __repConstruct(id,0);throw Error('representation Option');}
 if(t.Result){if(!v||Object.keys(v).length!==1)throw Error('representation Result');if(Object.hasOwn(v,'Ok'))return __repConstruct(id,0,[inner(t.Result[0],v.Ok)]);if(Object.hasOwn(v,'Err'))return __repConstruct(id,1,[inner(t.Result[1],v.Err)]);}
 throw Error('unsupported representation source type');
}
function __repDecode(c,t,v,depth=0){
 if(depth>32)throw Error('representation source depth');if(t==='U64'||t==='Bool')return __repEncode(c,t,v,depth);
 const id=__repTypeId(c,t),[tag,args]=__repData(v,id),inner=(t,v)=>__repDecode(c,t,v,depth+1);
 if(t==='Unit'&&tag===0&&!args.length)return null;
 if(t.List){const xs=[];let cur=v;while(true){const[tag,args]=__repData(cur,id);if(tag===0&&!args.length)return xs;if(tag!==1||args.length!==2||xs.length>=4096)throw Error('representation list carrier');xs.push(inner(t.List,args[0]));cur=args[1];}}
 if(t.Named){const name=t.Named;if(c.enums[name]&&!args.length){const variant=c.enums[name][tag];if(variant===undefined)throw Error('representation enum');return name+'.'+variant;}const fields=c.records[name];if(fields&&tag===0&&args.length===fields.length)return Object.fromEntries(fields.map(([n,t],i)=>[n,inner(t,args[i])]));}
 if(t.Option){if(tag===0&&!args.length)return {None:null};if(tag===1&&args.length===1)return {Some:inner(t.Option,args[0])};}
 if(t.Result&&tag<2&&args.length===1)return tag===0?{Ok:inner(t.Result[0],args[0])}:{Err:inner(t.Result[1],args[0])};
 throw Error('representation source shape');
}
function __repFromTables(c,tables){return __repConstruct(c.state,0,c.tables.map((model,i)=>{const rows=[...tables[i].entries()].sort(([a],[b])=>a<b?-1:a>b?1:0);if(rows.length>4096)throw Error('representation table limit');let tail=__repConstruct(model.datatype,0);for(let j=rows.length-1;j>=0;j--)tail=__repConstruct(model.datatype,1,[__repEncode(c,'U64',rows[j][0]),__repEncode(c,model.value_type,rows[j][1]),tail]);return tail;}));}
function __repTables(c,state){const[tag,roots]=__repData(state,c.state);if(tag!==0||roots.length!==c.tables.length)throw Error('representation state shape');return c.tables.map((model,i)=>{const rows=new Map();let cur=roots[i],prior=null;while(true){const[tag,args]=__repData(cur,model.datatype);if(tag===0&&!args.length)return rows;if(tag!==1||args.length!==3||rows.size>=4096)throw Error('representation table carrier');const key=__repDecode(c,'U64',args[0]);if(prior!==null&&prior>=key)throw Error('representation unordered keys');prior=key;rows.set(key,__repDecode(c,model.value_type,args[1]));cur=args[2];}});}
function __repEvents(c,list){const out=[];let cur=list;while(true){const[tag,args]=__repData(cur,c.events);if(tag===0&&!args.length)return out;if(tag!==1||args.length!==2||out.length>=4096)throw Error('representation event list');const[channel,fields]=__repData(args[0],c.event);if(fields.length!==1||!c.channels[channel])throw Error('representation event carrier');const[name,t]=c.channels[channel];out.push([name,__repDecode(c,t,fields[0])]);cur=args[1];}}
function __repPreflight(rt){const limits=__snapshotLimits(),w=__snapshotWriter(limits);w.integer(rt.version,8);const roots=__stateMetadata.roots.map((r,i)=>({...r,index:i})).sort((a,b)=>a.name<b.name?-1:1);for(const r of roots){const rows=[...rt.tables[r.index].entries()].sort(([a],[b])=>a<b?-1:a>b?1:0);w.integer(rows.length,8);for(const[k,v]of rows){w.value(k,r.key);w.value(v,r.value);}}w.integer(rt.outbox.length,8);for(const e of rt.outbox){const channel=__stateMetadata.events.findIndex(t=>t.name===e.channel);w.integer(e.commit,8);w.integer(e.position,8);w.integer(channel,4);w.value(e.value,__stateMetadata.events[channel].type);}if(w.finish().length+116>limits.bytes)throw Error('snapshot byte limit');}
export function createRepresentationState(id=__representationIds[0]){
 let c=__representationPlans[id];if(!c)throw Error('unknown compiled representation');
 const rt={tables:__stateMetadata.roots.map(()=>new Map()),version:0n,outbox:[],busy:false};let physical=__representationExecutors[id](c.encode,[__repFromTables(c,rt.tables)]);
 const stats={transitions:0,encodes:1,decodes:0,migrations:0};const boundary=()=>{if(rt.busy)throw Error('state operation already in progress');};
 const run=(fn,args)=>__representationExecutors[c.id](fn,args);
 function invoke(name,args){
  boundary();const a=c.actions[name],meta=__stateMetadata.actions.find(a=>a.name===name);if(!a||!meta||!Array.isArray(args)||args.length!==meta.params.length)throw Error('unknown action or argument count');
  const inputs=args.map((v,i)=>__repEncode(c,a.params[i],__stateInput(v,meta.params[i].type)));
  rt.busy=true;
  try{
   const executed=run(a.execute,[physical,rt.version,...inputs]);const observed=run(a.observe,[executed]),next=run(a.next,[executed]);stats.transitions++;
   const[tag,fields]=__repData(observed,a.observation);if(tag===1)throw Error('commit sequence exhausted');if(tag!==0||fields.length!==5)throw Error('representation observation shape');
   const[reply,committed,version,state,events]=fields;const result=__repDecode(c,a.result,reply),tables=__repTables(c,state),staged=__repEvents(c,events);if(typeof committed!=='boolean'||typeof version!=='bigint')throw Error('representation observation scalars');
   if(committed){if(version!==rt.version+1n||version>18446744073709551615n)throw Error('invalid commit sequence');}else if(version!==rt.version||staged.length)throw Error('invalid aborted/query observation');
   const pending=staged.map(([channel,value],i)=>({commit:version,position:BigInt(i),channel,value}));const outbox=rt.outbox.concat(pending);__repPreflight({tables,version,outbox});
   const published=pending.map(e=>({...e,value:__stateOutput(e.value,__stateMetadata.events.find(t=>t.name===e.channel).type)}));const output=__stateOutput(result,meta.result);
   physical=next;rt.tables=tables;rt.version=version;rt.outbox=outbox;return {result:output,committed,version,events:published};
  }finally{rt.busy=false;}
 }
 function switchTo(id,migration){boundary();const to=__representationPlans[id];if(!to||to.source!==c.source)throw Error('migration source mismatch');rt.busy=true;try{const logical=run(c.decode,[physical]);stats.decodes++;let next;if(migration){const m=__representationMigrations[migration];if(!m||m.from!==c.id||m.to!==id||m.source!==c.source)throw Error('migration endpoints');next=__representationExecutors[migration](m.function,[physical]);}else{next=__representationExecutors[id](to.encode,[logical]);stats.encodes++;}const decoded=__representationExecutors[id](to.decode,[next]);stats.decodes++;__repTables(to,decoded);if(!__repEqual(logical,decoded))throw Error('migration changed logical state');c=to;physical=next;stats.migrations++;}finally{rt.busy=false;}}
 return Object.freeze({invoke,invoke_json:invoke,get representation(){return c.id;},get version(){return rt.version;},get outbox(){return rt.outbox.map(e=>({...e,value:__stateOutput(e.value,__stateMetadata.events.find(t=>t.name===e.channel).type)}));},statistics(){return {...stats};},migrate:id=>switchTo(id),migrateDirect:(id,migration)=>switchTo(id,migration),
  acknowledgeThrough(commit,position){boundary();commit=__repEncode(c,'U64',BigInt(commit));position=__repEncode(c,'U64',BigInt(position));rt.outbox=rt.outbox.filter(e=>e.commit>commit||(e.commit===commit&&e.position>position));},
  checkpoint(limits){boundary();const logical=run(c.decode,[physical]);stats.decodes++;return __snapshotEncode(__stateMetadata,{...rt,tables:__repTables(c,logical)},limits);},
  async restore(bytes,limits){boundary();rt.busy=true;try{const state=await __snapshotDecode(__stateMetadata,bytes,limits);const logical=__repFromTables(c,state.tables),next=run(c.encode,[logical]);stats.encodes++;const decoded=run(c.decode,[next]);stats.decodes++;if(!__repEqual(logical,decoded))throw Error('restore changed logical state');physical=next;rt.tables=state.tables;rt.version=state.version;rt.outbox=state.outbox;}finally{rt.busy=false;}}
 });
}
