//! Narrow, checked Channel<int> guest source lowering.
use super::*;
impl<'a> FnCx<'a> {
    pub(super) fn channel_open(&mut self,args:&[Expr],pos:Pos)->R<TE>{
        if self.owner!=Owner::TopLevel {
            return Err(Diagnostic::new(pos,"E_UNSUPPORTED","Channel<int> guest ABI is Worker-local"));
        }
        self.arity("Channel<int>.bounded",1,args.len(),pos)?;
        let cap=match args[0].kind {
            ExprKind::Int(Some(n)) if n<=1024=>n as i32,
            _=>return Err(Diagnostic::new(args[0].pos,"E_UNSUPPORTED",
                "v1 Channel<int>.bounded requires a literal capacity 0..1024")),
        };
        self.imports.push(Import::ChannelOpen);
        let scratch=self.temp(Type::ChannelInt,pos)?;
        let status=self.temp(Type::IoError,pos)?;
        let result=self.temp(Type::ResultChannelIntIoError,pos)?;
        Ok(TE{e:TExpr::ChannelOpen{capacity:cap,scratch,status,result},
            ty:Type::ResultChannelIntIoError})
    }
    pub(super) fn channel_read(&mut self,args:&[Expr],pos:Pos)->R<TE>{
        if self.owner!=Owner::TopLevel||!self.suspends {
            return Err(Diagnostic::new(pos,"E_EFFECT",
                "nb readch requires susp fnc and Worker-owned Channel<int>"));
        }
        if self.state_write_scope||self.state_read_scope||!self.loans.is_empty(){
            return Err(Diagnostic::new(pos,"E_BORROW",
                "nb readch forbidden while a critical state or object loan is live"));
        }
        self.arity("nb readch",1,args.len(),pos)?;
        let channel=self.value(&args[0])?;
        if channel.ty!=Type::ChannelInt {
            return Err(type_err(args[0].pos,
                format!("nb readch expects Channel<int>, found {}",self.tn(channel.ty))));
        }
        self.imports.push(Import::ChannelNbRead);
        let scratch=self.temp(Type::ChannelInt,pos)?;
        let status=self.temp(Type::IoError,pos)?;
        Ok(TE{e:TExpr::ChannelRead{channel:Box::new(channel),scratch,status},
            ty:Type::FutureReadInt})
    }
    pub(super) fn channel_method(&mut self,channel:TE,name:&str,args:&[Expr],pos:Pos)->R<TE>{
        if self.owner!=Owner::TopLevel {
            return Err(Diagnostic::new(pos,"E_UNSUPPORTED","Channel<int> is Worker-local"));
        }
        match name {
            "send"=>{
                self.arity("Channel<int>.send",1,args.len(),pos)?;
                let value=self.value(&args[0])?;
                if value.ty!=Type::Int {
                    return Err(type_err(args[0].pos,"Channel<int>.send requires int"));
                }
                self.imports.push(Import::ChannelSend);
                let status=self.temp(Type::IoError,pos)?;
                let result=self.temp(Type::ResultVoidIoError,pos)?;
                Ok(TE{e:TExpr::ChannelSend{channel:Box::new(channel),value:Box::new(value),status,result},
                    ty:Type::ResultVoidIoError})
            }
            "close"|"release"=>{
                self.arity(&format!("Channel<int>.{name}"),0,args.len(),pos)?;
                let operation=if name=="close"{Import::ChannelClose}else{Import::ChannelRelease};
                self.imports.push(operation);
                let status=self.temp(Type::IoError,pos)?;
                let result=self.temp(Type::ResultVoidIoError,pos)?;
                Ok(TE{e:TExpr::ChannelFinish{channel:Box::new(channel),operation,status,result},
                    ty:Type::ResultVoidIoError})
            }
            _=>Err(type_err(pos,format!("Channel<int> has no method '{name}'"))),
        }
    }
}
