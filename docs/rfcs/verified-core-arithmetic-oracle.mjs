// HOST_REFERENCE_ONLY. No compiler integration or guest proof.
// Inputs are typed, inclusive signed intervals. Refuted means ALL values in
// the Cartesian abstraction fail; it is not a reachability proof. Unknown
// must not remove a check. Callers must validate SSA identity and reachability.
function classify(op,a,b,width=64){
  if (!Number.isInteger(width)||width<2||width>64) throw new RangeError("width");
  const min=-(1n<<BigInt(width-1)),max=-min-1n;
  const valid=r=>Array.isArray(r)&&r.length===2&&r.every(v=>typeof v==="bigint"&&v>=min&&v<=max)&&r[0]<=r[1];
  if(!valid(a)|| (op!=="neg"&&!valid(b))) return "unknown";
  const [l,h]=a;
  if(op==="neg") return l>min?"proven":h===min?"refuted":"unknown";
  const [m,n]=b;
  if(op==="div"){
    const zero=m<=0n&&n>=0n, overflow=l===min&&m<=-1n&&n>=-1n;
    if(!zero&&!overflow)return "proven";
    if(m===0n&&n===0n || l===min&&h===min&&m===-1n&&n===-1n)return "refuted";
    return "unknown";
  }
  let v;
  switch(op){
    case "add":v=[l+m,h+n];break;
    case "sub":v=[l-n,h-m];break;
    case "mul":v=[l*m,l*n,h*m,h*n];break;
    default:return "unknown";
  }
  const lo=v.reduce((x,y)=>x<y?x:y),hi=v.reduce((x,y)=>x>y?x:y);
  return lo>=min&&hi<=max?"proven":hi<min||lo>max?"refuted":"unknown";
}

function run(){
  let checks=0,intervalPairs=0,concrete=0;
  const assert=(p,msg)=>{checks++;if(!p)throw Error(msg)};
  const oracle=(op,x,y,min,max)=>{
    if(op==="div"&&y===0n)return false;
    const v=op==="add"?x+y:op==="sub"?x-y:op==="mul"?x*y:op==="div"?x/y:-x;
    return v>=min&&v<=max;
  };
  const intervals=[];for(let l=-8n;l<=7n;l++)for(let h=l;h<=7n;h++)intervals.push([l,h]);
  for(const a of intervals)for(const b of intervals){
    intervalPairs++;
    for(const op of ["add","sub","mul","div"]){
      const s=classify(op,a,b,4);let safe=0,unsafe=0;
      for(let x=a[0];x<=a[1];x++)for(let y=b[0];y<=b[1];y++){concrete++;if(oracle(op,x,y,-8n,7n))safe++;else unsafe++}
      assert(s!=="proven"||unsafe===0,"false proof "+op);
      assert(s!=="refuted"||safe===0,"false universal refutation "+op);
      assert(safe===0||unsafe===0||s==="unknown","mixed range "+op);
    }
  }
  for(const a of intervals){const s=classify("neg",a,null,4);for(let x=a[0];x<=a[1];x++){concrete++;const safe=oracle("neg",x,0n,-8n,7n);assert(s!=="proven"||safe,"neg false proof");assert(s!=="refuted"||!safe,"neg false refutation")}}
  const min=-(1n<<63n),max=-min-1n;
  const edges=[
    ["div",[min,min],[-1n,-1n],"refuted"],
    ["div",[min,max],[1n,1n],"proven"],
    ["div",[min,max],[-2n,-1n],"unknown"],
    ["div",[min+1n,max],[-2n,-1n],"proven"],
    ["div",[0n,0n],[-1n,1n],"unknown"],
    ["div",[1n,1n],[0n,0n],"refuted"],
    ["add",[max,max],[1n,1n],"refuted"],
    ["add",[max-1n,max],[0n,1n],"unknown"],
    ["sub",[min,min],[1n,1n],"refuted"],
    ["mul",[min,min],[-1n,-1n],"refuted"],
    ["mul",[min,max],[0n,0n],"proven"],
    ["neg",[min,min],null,"refuted"],
    ["neg",[min+1n,max],null,"proven"],
    ["add",[0n,0n],[max,max],"proven"],
    ["add",[1n,0n],[0n,0n],"unknown"],
    ["add",[min-1n,min],[0n,0n],"unknown"],
    ["unsupported",[0n,0n],[0n,0n],"unknown"]
  ];
  for(const [op,a,b,s] of edges)assert(classify(op,a,b)===s,"edge "+op+" expected "+s);
  return {checks,intervalPairs,concrete,edgeCases:edges.length};
}

console.log(JSON.stringify(run()));
