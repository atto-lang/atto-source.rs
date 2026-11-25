//! Executes the shared nominal-kind matrix against production resolver APIs.
//! The synthetic authenticated views do not claim package loading or Wasm ABI evidence.
use atto_compiler::{Diagnostic, Pos, import_v2::{self,Kind,Source}, import_tree,
    lexer::{lex,Tok,Token},static_import_graph::{ExportView,Origin,PackageId,Scope}};

type R<T>=Result<T,Diagnostic>;
fn word(t:Option<&Token>,s:&str)->bool{matches!(t.map(|t|&t.tok),Some(Tok::Ident(w)) if w==s)}
fn syntax()->Diagnostic{Diagnostic::new(Pos::default(),"E_SYNTAX","invalid matrix adapter fixture")}
fn path(tokens:&[Token])->R<String>{
    if tokens.is_empty()||tokens.len()%2==0{return Err(syntax());}
    let mut out=String::new();
    for (i,t) in tokens.iter().enumerate(){
        match (&t.tok,i%2){
            (Tok::Ident(s),0)=>out.push_str(s),
            (Tok::Dot,1)=>out.push('.'),
            _=>return Err(syntax()),
        }
    }
    Ok(out)
}
fn kind(s:&str)->Kind{match s{
    "class"=>Kind::Class,"enum"=>Kind::Enum,"actor"=>Kind::Actor,"type"=>Kind::Type,
    "fnc"=>Kind::Fnc,"namespace"=>Kind::Namespace,_=>panic!("unknown fixture kind {s}")
}}

fn resolve(scope:&mut Scope,source:&str,reference:&str,packages:&[(String,PackageId,ExportView)]) -> R<atto_compiler::static_import_graph::Target>{
    let tokens=lex(source)?;
    let views:Vec<_>=packages.iter().map(|(_,_,v)|v).collect();
    let mut i=0;
    let mut aliases=Vec::new();
    while tokens.get(i).is_some_and(|t|t.tok!=Tok::Eof){
        let end=tokens[i..].iter().position(|t|t.tok==Tok::Semi).map(|j|i+j).ok_or_else(syntax)?;
        if word(tokens.get(i),"import") {
            if word(tokens.get(i+1),"pkg"){
                let name=match tokens.get(i+2).map(|t|&t.tok){Some(Tok::Str(s))=>s,_=>return Err(syntax())};
                let (_,id,_)=packages.iter().find(|(n,_,_)|n==name).ok_or_else(syntax)?;
                let alias=if i+3==end {None}else{
                    if !word(tokens.get(i+3),"as"){return Err(syntax());}
                    Some(path(&tokens[i+4..end])?)
                };
                scope.package(id.clone(),alias.as_deref())?;
            }else if import_v2::is_mixed(&tokens,i+1){
                let (m,next)=import_v2::parse_mixed(&tokens,i+1)?;
                if next!=end+1{return Err(syntax());}
                let Source::Package(name)=&m.source else{return Err(syntax())};
                let (_,_,view)=packages.iter().find(|(n,_,_)|n==name).ok_or_else(syntax)?;
                scope.bind_mixed(view,&m)?;
            }else{
                let (leaves,next)=import_tree::parse(&tokens,i+1)?;
                if next!=end+1{return Err(syntax());}
                import_tree::bind(scope,&leaves,&views)?;
            }
        }else if word(tokens.get(i),"define")&&word(tokens.get(i+1),"alias"){
            // The matrix is a resolver test. Decode only the two fixture
            // alias forms and use the real transactional graph binder.
            let sep=(i+2..end).find(|&j|tokens[j].tok==Tok::Assign||word(tokens.get(j),"as")).ok_or_else(syntax)?;
            let a=path(&tokens[i+2..sep])?;
            let b=path(&tokens[sep+1..end])?;
            if tokens[sep].tok==Tok::Assign{aliases.push((a,b));}else{aliases.push((b,a));}
        }else{return Err(syntax());}
        i=end+1;
    }
    let pairs:Vec<_>=aliases.iter().map(|(n,r)|(n.as_str(),r.as_str())).collect();
    scope.namespace_aliases(&pairs,&views)?;
    scope.resolve_in_views(reference,&views)
}

#[test]
fn shared_nominal_matrix(){
    let content=include_str!("shared-import-v2/nominal-matrix.tsv");
    let mut packages=Vec::new();
    let mut exports=Vec::new();
    let mut cases=Vec::new();
    for line in content.lines(){
        let cols:Vec<_>=line.split('\t').collect();
        match cols.as_slice(){
            ["package",name,digest]=>packages.push((name.to_string(),PackageId::new(name,"1.0.0",digest).unwrap())),
            ["export",name,k,public]=>exports.push((*name,kind(k),*public=="true")),
            ["case",id,source,reference,outcome,identity,declaration,code,atomic]=>cases.push((*id,source.replace("\\n","\n"),*reference,*outcome,*identity,*declaration,*code,*atomic=="true")),
            _=>panic!("invalid projection line {line}"),
        }
    }
    assert_eq!(packages.len(),2);
    assert_eq!(cases.len(),35);
    let views:Vec<_>=packages.into_iter().map(|(name,id)|{
        let mut view=ExportView::new(Origin::External(id.clone()));
        for (n,k,pub_) in &exports{view.add(n,n,*k,*pub_).unwrap();}
        (name,id,view)
    }).collect();
    for (id,source,reference,outcome,identity,declaration,code,atomic) in cases {
        let mut scope=Scope::new();
        let actual=resolve(&mut scope,&source,reference,&views);
        if outcome=="identity" {
            let t=actual.unwrap_or_else(|e|panic!("{id}: {e}"));
            let (_,expected,view)=views.iter().find(|(n,_,_)|n==identity).unwrap();
            assert_eq!(t.origin,Origin::External(expected.clone()),"{id}");
            assert_eq!(t.declaration,declaration,"{id}");
            assert_eq!(t,view.lookup(declaration).unwrap(),"{id}");
        }else{
            assert_eq!(actual.unwrap_err().code,code,"{id}");
            if atomic{
                assert!(scope.resolve(reference).is_err(),"{id}: partially bound rejected selection");
                assert!(scope.resolve("Denied").is_err(),"{id}: leaked denied selection");
            }
        }
        println!("{id}: PASS");
    }
}
