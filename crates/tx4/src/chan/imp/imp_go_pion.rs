use crate::*;

pub struct ImpChan {
    chan: tx4_go_pion::DataChannel,
    _not_sync: std::marker::PhantomData<std::cell::Cell<()>>,
}

impl ImpChan {
    pub fn label(&mut self) -> Result<Vec<u8>> {
        self.chan.label()?.to_vec()
    }

    pub fn is_closed(&mut self) -> Result<bool> {
        Ok(self.chan.ready_state()? > 2)
    }

    pub async fn send<'a, B>(&mut self, data: B) -> Result<()>
    where
        B: Into<&'a mut Buf>,
    {
        let data = data.into();
        self.chan.send(&mut data.imp.buf).await
    }
}
