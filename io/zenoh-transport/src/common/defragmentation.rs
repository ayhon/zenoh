//
// Copyright (c) 2022 ZettaScale Technology
//
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// http://www.eclipse.org/legal/epl-2.0, or the Apache License, Version 2.0
// which is available at https://www.apache.org/licenses/LICENSE-2.0.
//
// SPDX-License-Identifier: EPL-2.0 OR Apache-2.0
//
// Contributors:
//   ZettaScale Zenoh Team, <zenoh@zettascale.tech>
//
use super::seq_num::SeqNum;
use zenoh_buffers::{reader::HasReader, SplitBuffer, ZBuf, ZSlice};
use zenoh_codec::{RCodec, Zenoh060Reliability};
use zenoh_core::{bail, Result as ZResult};
use zenoh_protocol::{
    core::{Reliability, ZInt},
    zenoh::ZenohMessage,
};

#[derive(Debug)]
pub(crate) struct DefragBuffer {
    reliability: Reliability,
    pub(crate) sn: SeqNum,
    capacity: usize,
    len: usize,
    buffer: ZBuf,
}

impl DefragBuffer {
    pub(crate) fn make(
        reliability: Reliability,
        sn_resolution: ZInt,
        capacity: usize,
    ) -> ZResult<DefragBuffer> {
        let db = DefragBuffer {
            reliability,
            sn: SeqNum::make(0, sn_resolution)?,
            capacity,
            len: 0,
            buffer: ZBuf::default(),
        };
        Ok(db)
    }

    #[inline(always)]
    pub(crate) fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    #[inline(always)]
    pub(crate) fn clear(&mut self) {
        self.len = 0;
        self.buffer.clear();
    }

    #[inline(always)]
    pub(crate) fn sync(&mut self, sn: ZInt) -> ZResult<()> {
        self.sn.set(sn)
    }

    pub(crate) fn push(&mut self, sn: ZInt, zslice: ZSlice) -> ZResult<()> {
        if sn != self.sn.get() {
            self.clear();
            bail!("Expected SN {}, received {}", self.sn.get(), sn)
        }

        self.len += zslice.len();
        if self.len > self.capacity {
            self.clear();
            bail!(
                "Defragmentation buffer full: {} bytes. Capacity: {}.",
                self.len,
                self.capacity
            )
        }

        self.buffer.push_zslice(zslice);
        self.sn.increment();

        Ok(())
    }

    #[inline(always)]
    pub(crate) fn defragment(&mut self) -> Option<ZenohMessage> {
        let mut reader = self.buffer.reader();
        let rcodec = Zenoh060Reliability::new(self.reliability);
        let res: Option<ZenohMessage> = rcodec.read(&mut reader).ok();
        self.buffer.clear();
        res
    }
}
