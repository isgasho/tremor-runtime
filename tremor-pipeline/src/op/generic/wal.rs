// Copyright 2018-2020, Wayfair GmbH
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::op::prelude::*;
use std::mem;
use tremor_script::prelude::*;

const OUT: Cow<'static, str> = Cow::Borrowed("out");

#[derive(Debug, Clone)]
// TODO add seed value and field name as config items
pub struct WAL {
    wal: sled::Db,
    write: usize,
    read: usize,
    read_count: usize,
    broken: bool,
}

op!(WalFactory(_node) {
    let wal = sled::open("./wal")?;
    Ok(Box::new(WAL{
        wal,
        read: 1,
        write: 0,
        read_count: 5,
        broken: false
    }))
});

impl WAL {
    fn read_events(&mut self) -> Result<Vec<(Cow<'static, str>, Event)>> {
        // get the ID where to read from
        let read_buf: [u8; 8] = unsafe { mem::transmute(self.read.to_be()) };
        // The maximum number of entries we read
        let read_end = self.read + self.read_count;
        let read_end_buf: [u8; 8] = unsafe { mem::transmute(read_end.to_be()) };
        let mut events = Vec::with_capacity(self.read_count);
        for e in self.wal.range(read_buf..=read_end_buf).values() {
            self.read += 1;
            let e_slice: &[u8] = &e?;
            let mut ev = Vec::from(e_slice);
            let event = simd_json::from_slice(&mut ev)?;
            events.push((OUT, event))
        }
        Ok(events)
    }
}
#[allow(unused_mut)]
impl Operator for WAL {
    fn on_event(
        &mut self,
        _port: &str,
        _state: &mut Value<'static>,
        event: Event,
    ) -> Result<Vec<(Cow<'static, str>, Event)>> {
        // Calculate the next Id to write to
        self.write += 1;
        let write_buf: [u8; 8] = unsafe { mem::transmute(self.write.to_be()) };

        // Sieralize and write the event
        let event_buf = simd_json::serde::to_vec(&event)?;
        self.wal.insert(write_buf, event_buf)?;
        if self.broken {
            Ok(vec![])
        } else {
            self.read_events()
        }
    }
}
