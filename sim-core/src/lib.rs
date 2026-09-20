//! 無畫面模擬核心原型（headless simulation core prototype）
//!
//! 設計要點
//! - 全整數：時間是 tick（u32），輸送帶位置是「格」（cell，= 物品間距），沒有任何浮點數。
//! - 事件驅動：沒有 per-tick 的全物件掃描。每個節點只在「有事發生」時被叫醒（timing wheel）。
//! - 輸送帶 = 模型 B：環形緩衝區語意（只動一個整數偏移）+ RLE 區段（item, count, gap）。
//!   堵住時整條停住（不壓縮），所以物品的相對位置固定，移動只是把 gap0 減一，而且是「懶惰結算」的：
//!   真正的計算發生在有人來問的時候（sync），平時一個 tick 都不用碰。
//! - 休眠：被堵住的節點沒有計時器，成本為零；上游堵住時登記在下游的等待串列上，下游一有進展就叫醒。

use std::cmp::Reverse;
use std::collections::BinaryHeap;

pub const NONE: u32 = u32::MAX;

const KIND_SHIFT: u32 = 30;
const IDX_MASK: u32 = (1 << KIND_SHIFT) - 1;
pub const K_BELT: u32 = 0;
pub const K_MACH: u32 = 1;
pub const K_SINK: u32 = 2;
pub const K_SRC: u32 = 3;

#[inline(always)]
pub fn mkid(kind: u32, idx: u32) -> u32 {
    (kind << KIND_SHIFT) | idx
}
#[inline(always)]
pub fn kind_of(id: u32) -> u32 {
    id >> KIND_SHIFT
}
#[inline(always)]
pub fn idx_of(id: u32) -> usize {
    (id & IDX_MASK) as usize
}

const F_BLOCKED: u8 = 1; // 輸出被堵住（輸送帶頭端 / 機器產物 / 產生器）
const F_WAIT: u8 = 2; // 目前掛在某個節點的等待串列上
const F_WHEEL: u8 = 4; // 目前在 timing wheel 裡
const F_BUSY: u8 = 8; // 機器加工中
const F_HASOUT: u8 = 16; // 機器已產出但還沒送出去
const F_READY: u8 = 32; // 這個 tick 內待處理

const SLOTS: usize = 1 << 16;
const SLOT_MASK: u32 = (SLOTS as u32) - 1;

/// 輸送帶。36 bytes。
/// 格子編號 0 = 頭端（輸出端），cap-1 = 尾端（輸入端）。
/// `gap0` = 頭端到第一個物品之間的空格數；`used` = 最後一個物品的位置 + 1。
/// 尾端還能塞進東西 <=> used < cap。
#[repr(C)]
#[derive(Clone)]
pub struct Belt {
    pub out: u32,
    run_head: u32,
    run_tail: u32,
    /// 上一次「結算」的 tick（環形緩衝區的偏移基準點）
    last: u32,
    /// 同時只會在 wheel 或等待串列其中之一，所以共用一個欄位
    link: u32,
    wait_head: u32,
    pub cap: u16,
    pub period: u16, // 前進一格要幾個 tick
    gap0: u16,
    used: u16,
    flags: u8,
}

/// RLE 區段：連續 `count` 個相同物品，前面有 `gap` 個空格。12 bytes。
#[repr(C)]
#[derive(Clone)]
struct Run {
    next: u32,
    item: u16,
    count: u16,
    gap: u16,
}

/// 機器。28 bytes。
#[repr(C)]
#[derive(Clone)]
pub struct Machine {
    pub out: u32,
    until: u32,
    link: u32,
    wait_head: u32,
    in_item: u16,
    out_item: u16,
    dur: u16,
    need: u8,
    have: u8,
    flags: u8,
}

#[repr(C)]
#[derive(Clone)]
pub struct Sink {
    pub count: u64,
    wait_head: u32,
    pub open: u8,
}

#[repr(C)]
#[derive(Clone)]
pub struct Source {
    pub out: u32,
    until: u32,
    link: u32,
    period: u16,
    item: u16,
    flags: u8,
}

pub struct World {
    pub now: u32,
    pub belts: Vec<Belt>,
    pub machines: Vec<Machine>,
    pub sinks: Vec<Sink>,
    pub sources: Vec<Source>,
    runs: Vec<Run>,
    run_free: Vec<u32>,
    wheel: Vec<u32>,
    overflow: BinaryHeap<Reverse<(u32, u32)>>,
    ready: Vec<u32>,
    batch: Vec<u32>,
    batch2: Vec<u32>,
    /// 實驗用：事件先收集起來按 id 排序再處理（想把隨機存取變成循序存取）。
    /// 實測在 10⁷ 規模只有 ~5% 差異、在 10⁶ 規模反而更慢，所以預設關閉。
    pub sort_events: bool,
    /// 統計：處理過的事件總數
    pub events: u64,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        World {
            now: 0,
            belts: Vec::new(),
            machines: Vec::new(),
            sinks: Vec::new(),
            sources: Vec::new(),
            runs: Vec::new(),
            run_free: Vec::new(),
            wheel: vec![NONE; SLOTS],
            overflow: BinaryHeap::new(),
            ready: Vec::new(),
            batch: Vec::new(),
            batch2: Vec::new(),
            sort_events: false,
            events: 0,
        }
    }

    pub fn reserve(&mut self, belts: usize, machines: usize, sinks: usize, sources: usize) {
        self.belts.reserve_exact(belts);
        self.machines.reserve_exact(machines);
        self.sinks.reserve_exact(sinks);
        self.sources.reserve_exact(sources);
        self.runs.reserve_exact(belts + 8);
    }

    // ---------------------------------------------------------------- 建構

    pub fn add_belt(&mut self, cap: u16, period: u16, out: u32) -> u32 {
        assert!(cap >= 1 && period >= 1);
        self.belts.push(Belt {
            out,
            run_head: NONE,
            run_tail: NONE,
            last: self.now,
            link: NONE,
            wait_head: NONE,
            cap,
            period,
            gap0: 0,
            used: 0,
            flags: 0,
        });
        mkid(K_BELT, (self.belts.len() - 1) as u32)
    }

    pub fn add_machine(&mut self, in_item: u16, need: u8, out_item: u16, dur: u16, out: u32) -> u32 {
        assert!(need >= 1 && dur >= 1);
        self.machines.push(Machine {
            out,
            until: 0,
            link: NONE,
            wait_head: NONE,
            in_item,
            out_item,
            dur,
            need,
            have: 0,
            flags: 0,
        });
        mkid(K_MACH, (self.machines.len() - 1) as u32)
    }

    pub fn add_sink(&mut self) -> u32 {
        self.sinks.push(Sink { count: 0, wait_head: NONE, open: 1 });
        mkid(K_SINK, (self.sinks.len() - 1) as u32)
    }

    pub fn add_source(&mut self, item: u16, period: u16, out: u32) -> u32 {
        assert!(period >= 1);
        self.sources.push(Source { out, until: 0, link: NONE, period, item, flags: 0 });
        let id = mkid(K_SRC, (self.sources.len() - 1) as u32);
        self.schedule(id, self.now);
        id
    }

    pub fn set_belt_out(&mut self, belt: u32, out: u32) {
        self.belts[idx_of(belt)].out = out;
    }

    pub fn node_count(&self) -> usize {
        self.belts.len() + self.machines.len() + self.sinks.len() + self.sources.len()
    }

    // ---------------------------------------------------------- 計時器 / 串列

    #[inline]
    fn link_of(&self, id: u32) -> u32 {
        match kind_of(id) {
            K_BELT => self.belts[idx_of(id)].link,
            K_MACH => self.machines[idx_of(id)].link,
            K_SRC => self.sources[idx_of(id)].link,
            _ => NONE,
        }
    }

    #[inline]
    fn set_link(&mut self, id: u32, v: u32) {
        match kind_of(id) {
            K_BELT => self.belts[idx_of(id)].link = v,
            K_MACH => self.machines[idx_of(id)].link = v,
            K_SRC => self.sources[idx_of(id)].link = v,
            _ => {}
        }
    }

    #[inline]
    fn flags_of(&self, id: u32) -> u8 {
        match kind_of(id) {
            K_BELT => self.belts[idx_of(id)].flags,
            K_MACH => self.machines[idx_of(id)].flags,
            K_SRC => self.sources[idx_of(id)].flags,
            _ => 0,
        }
    }

    #[inline]
    fn or_flags(&mut self, id: u32, f: u8) {
        match kind_of(id) {
            K_BELT => self.belts[idx_of(id)].flags |= f,
            K_MACH => self.machines[idx_of(id)].flags |= f,
            K_SRC => self.sources[idx_of(id)].flags |= f,
            _ => {}
        }
    }

    #[inline]
    fn clear_flags(&mut self, id: u32, f: u8) {
        match kind_of(id) {
            K_BELT => self.belts[idx_of(id)].flags &= !f,
            K_MACH => self.machines[idx_of(id)].flags &= !f,
            K_SRC => self.sources[idx_of(id)].flags &= !f,
            _ => {}
        }
    }

    #[inline]
    fn schedule(&mut self, id: u32, when: u32) {
        debug_assert!(self.flags_of(id) & (F_WAIT | F_WHEEL) == 0, "已經在某個串列裡");
        if when <= self.now {
            if self.flags_of(id) & F_READY == 0 {
                self.or_flags(id, F_READY);
                self.ready.push(id);
            }
            return;
        }
        self.or_flags(id, F_WHEEL);
        if when - self.now >= SLOTS as u32 {
            self.overflow.push(Reverse((when, id)));
            return;
        }
        let slot = (when & SLOT_MASK) as usize;
        self.set_link(id, self.wheel[slot]);
        self.wheel[slot] = id;
    }

    /// 把 `waiter` 掛到 `target` 的等待串列上（target 有進展時會叫醒它）
    fn push_waiter(&mut self, target: u32, waiter: u32) {
        if self.flags_of(waiter) & F_WAIT != 0 {
            return;
        }
        self.or_flags(waiter, F_WAIT);
        let head = match kind_of(target) {
            K_BELT => &mut self.belts[idx_of(target)].wait_head,
            K_MACH => &mut self.machines[idx_of(target)].wait_head,
            K_SINK => &mut self.sinks[idx_of(target)].wait_head,
            _ => return,
        };
        let old = *head;
        *head = waiter;
        self.set_link(waiter, old);
    }

    /// 叫醒 target 上所有等待者（這個 tick 內就重試）
    fn wake_waiters(&mut self, target: u32) {
        let head = match kind_of(target) {
            K_BELT => std::mem::replace(&mut self.belts[idx_of(target)].wait_head, NONE),
            K_MACH => std::mem::replace(&mut self.machines[idx_of(target)].wait_head, NONE),
            K_SINK => std::mem::replace(&mut self.sinks[idx_of(target)].wait_head, NONE),
            _ => NONE,
        };
        let mut cur = head;
        while cur != NONE {
            let next = self.link_of(cur);
            self.clear_flags(cur, F_WAIT);
            self.set_link(cur, NONE);
            if self.flags_of(cur) & F_READY == 0 {
                self.or_flags(cur, F_READY);
                self.ready.push(cur);
            }
            cur = next;
        }
    }

    /// 叫醒等待「輸送帶 i 的尾端空位」的節點：空位會在下一次前進時出現，直接排在那個 tick。
    fn wake_waiters_on_moving_belt(&mut self, i: usize) {
        let head = std::mem::replace(&mut self.belts[i].wait_head, NONE);
        if head == NONE {
            return;
        }
        let when = (self.belts[i].last + self.belts[i].period as u32).max(self.now + 1);
        let mut cur = head;
        while cur != NONE {
            let next = self.link_of(cur);
            self.clear_flags(cur, F_WAIT);
            self.set_link(cur, NONE);
            self.schedule(cur, when);
            cur = next;
        }
    }

    /// `pusher` 塞不進 `target`：要嘛登記等待，要嘛算出下一次可能成功的時間再來
    fn block_on(&mut self, pusher: u32, target: u32) {
        if kind_of(target) == K_BELT {
            let b = &self.belts[idx_of(target)];
            if b.flags & F_BLOCKED == 0 && b.run_head != NONE {
                // 輸送帶還在跑，只是滿了 -> 下一次前進就會空出一格
                let when = (b.last + b.period as u32).max(self.now + 1);
                self.schedule(pusher, when);
                return;
            }
        }
        self.push_waiter(target, pusher);
    }

    // ---------------------------------------------------------------- 輸送帶

    #[inline]
    fn alloc_run(&mut self, item: u16, count: u16, gap: u16) -> u32 {
        if let Some(idx) = self.run_free.pop() {
            self.runs[idx as usize] = Run { next: NONE, item, count, gap };
            idx
        } else {
            self.runs.push(Run { next: NONE, item, count, gap });
            (self.runs.len() - 1) as u32
        }
    }

    /// 把「時間過去了多少格」結算進資料裡。這是環形緩衝區偏移的懶惰版本：平時完全不用碰。
    #[inline]
    fn belt_sync(&mut self, i: usize) {
        let b = &mut self.belts[i];
        if b.flags & F_BLOCKED != 0 || b.gap0 == 0 {
            return;
        }
        let p = b.period as u32;
        let mut moved = (self.now - b.last) / p;
        if moved == 0 {
            return;
        }
        if moved > b.gap0 as u32 {
            moved = b.gap0 as u32;
        }
        b.gap0 -= moved as u16;
        b.used -= moved as u16;
        b.last += moved * p;
    }

    /// 從尾端塞入一個物品（呼叫端要先 sync）
    #[inline]
    fn belt_insert_tail(&mut self, i: usize, item: u16) -> bool {
        let b = &mut self.belts[i];
        if b.used >= b.cap {
            return false;
        }
        let gap = b.cap - b.used - 1;
        let was_empty = b.run_head == NONE;
        let tail = b.run_tail;
        b.used = b.cap;
        if !was_empty && gap == 0 {
            let r = &mut self.runs[tail as usize];
            if r.item == item && r.count < u16::MAX {
                r.count += 1;
                return true;
            }
        }
        let new = self.alloc_run(item, 1, gap);
        let b = &mut self.belts[i];
        if was_empty {
            b.run_head = new;
            b.run_tail = new;
            b.gap0 = gap;
            b.last = self.now;
            let when = self.now + gap as u32 * b.period as u32;
            let id = mkid(K_BELT, i as u32);
            self.schedule(id, when);
        } else {
            self.runs[tail as usize].next = new;
            self.belts[i].run_tail = new;
        }
        true
    }

    /// 頭端物品被取走
    #[inline]
    fn belt_pop_head(&mut self, i: usize) {
        let b = &mut self.belts[i];
        let h = b.run_head;
        let r = &mut self.runs[h as usize];
        r.count -= 1;
        if r.count == 0 {
            let next = r.next;
            self.run_free.push(h);
            let b = &mut self.belts[i];
            b.run_head = next;
            if next == NONE {
                b.run_tail = NONE;
                b.gap0 = 0;
                b.used = 0;
            } else {
                b.gap0 = self.runs[next as usize].gap + 1;
            }
        } else {
            // 後面那個物品原本緊貼著，現在距離頭端一格
            b.gap0 = 1;
        }
    }

    fn belt_fire(&mut self, i: usize) {
        self.belt_sync(i);
        let b = &self.belts[i];
        if b.run_head == NONE {
            return;
        }
        if b.gap0 != 0 {
            // 還沒到頭端（可能是舊的計時器），重排
            let when = b.last + b.gap0 as u32 * b.period as u32;
            let id = mkid(K_BELT, i as u32);
            self.schedule(id, when);
            return;
        }
        let item = self.runs[b.run_head as usize].item;
        let out = b.out;
        if self.try_push(out, item) {
            self.belt_pop_head(i);
            let b = &mut self.belts[i];
            b.flags &= !F_BLOCKED;
            b.last = self.now;
            let (gap0, empty, period) = (b.gap0, b.run_head == NONE, b.period);
            if !empty {
                let id = mkid(K_BELT, i as u32);
                self.schedule(id, self.now + gap0 as u32 * period as u32);
            }
            // 尾端的空位會在下一次前進時出現
            self.wake_waiters_on_moving_belt(i);
        } else {
            let b = &mut self.belts[i];
            b.flags |= F_BLOCKED;
            b.last = self.now;
            let id = mkid(K_BELT, i as u32);
            self.block_on(id, out);
        }
    }

    // ---------------------------------------------------------------- 機器

    fn mach_try_start(&mut self, i: usize) {
        let m = &mut self.machines[i];
        if m.flags & (F_BUSY | F_HASOUT) != 0 || m.have < m.need {
            return;
        }
        m.have -= m.need;
        m.flags |= F_BUSY;
        m.until = self.now + m.dur as u32;
        let (until, id) = (m.until, mkid(K_MACH, i as u32));
        self.schedule(id, until);
        self.wake_waiters(id); // 輸入槽空出來了
    }

    fn mach_fire(&mut self, i: usize) {
        let m = &self.machines[i];
        let id = mkid(K_MACH, i as u32);
        if m.flags & F_BUSY != 0 {
            if m.until != self.now {
                return; // 過期的計時器
            }
            let (out, item) = (m.out, m.out_item);
            self.machines[i].flags &= !F_BUSY;
            if self.try_push(out, item) {
                self.mach_try_start(i);
            } else {
                self.machines[i].flags |= F_HASOUT;
                self.block_on(id, out);
            }
        } else if m.flags & F_HASOUT != 0 {
            let (out, item) = (m.out, m.out_item);
            if self.try_push(out, item) {
                self.machines[i].flags &= !F_HASOUT;
                self.mach_try_start(i);
            } else {
                self.block_on(id, out);
            }
        } else {
            self.mach_try_start(i);
        }
    }

    // ---------------------------------------------------------------- 產生器

    fn src_fire(&mut self, i: usize) {
        let s = &self.sources[i];
        let (out, item, period) = (s.out, s.item, s.period);
        let id = mkid(K_SRC, i as u32);
        if self.try_push(out, item) {
            self.sources[i].flags &= !F_BLOCKED;
            self.sources[i].until = self.now + period as u32;
            self.schedule(id, self.now + period as u32);
        } else {
            self.sources[i].flags |= F_BLOCKED;
            self.block_on(id, out);
        }
    }

    // ---------------------------------------------------------------- 推送

    #[inline]
    fn try_push(&mut self, target: u32, item: u16) -> bool {
        match kind_of(target) {
            K_BELT => {
                let i = idx_of(target);
                self.belt_sync(i);
                self.belt_insert_tail(i, item)
            }
            K_MACH => {
                let i = idx_of(target);
                let m = &mut self.machines[i];
                if m.have >= m.need || m.in_item != item {
                    return false;
                }
                m.have += 1;
                if m.have >= m.need {
                    self.mach_try_start(i);
                }
                true
            }
            K_SINK => {
                let s = &mut self.sinks[idx_of(target)];
                if s.open == 0 {
                    return false;
                }
                s.count += 1;
                true
            }
            _ => false,
        }
    }

    pub fn set_sink_open(&mut self, sink: u32, open: bool) {
        let s = &mut self.sinks[idx_of(sink)];
        let was = s.open != 0;
        s.open = open as u8;
        if open && !was {
            self.wake_waiters(sink);
        }
    }

    // ---------------------------------------------------------------- 主迴圈

    pub fn step(&mut self) {
        let t = self.now;
        while let Some(&Reverse((tick, id))) = self.overflow.peek() {
            if tick > t {
                break;
            }
            self.overflow.pop();
            self.clear_flags(id, F_WHEEL);
            self.fire(id);
        }
        let slot = (t & SLOT_MASK) as usize;
        let mut cur = std::mem::replace(&mut self.wheel[slot], NONE);
        if self.sort_events {
            let mut batch = std::mem::take(&mut self.batch);
            batch.clear();
            while cur != NONE {
                let next = self.link_of(cur);
                self.set_link(cur, NONE);
                self.clear_flags(cur, F_WHEEL);
                batch.push(cur);
                cur = next;
            }
            if batch.len() > 4096 {
                let mut scratch = std::mem::take(&mut self.batch2);
                radix_sort(&mut batch, &mut scratch);
                self.batch2 = scratch;
            }
            for k in 0..batch.len() {
                self.fire(batch[k]);
            }
            self.batch = batch;
        } else {
            while cur != NONE {
                let next = self.link_of(cur);
                self.set_link(cur, NONE);
                self.clear_flags(cur, F_WHEEL);
                self.fire(cur);
                cur = next;
            }
        }
        while let Some(id) = self.ready.pop() {
            self.clear_flags(id, F_READY);
            self.fire(id);
        }
        self.now = t + 1;
    }

    pub fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step();
        }
    }

    /// 預取下一個要處理的節點：事件驅動的成本幾乎全在 cache miss 上
    #[inline(always)]
    fn prefetch(&self, id: u32) {
        #[cfg(target_arch = "x86_64")]
        {
            if id == NONE {
                return;
            }
            unsafe {
                use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
                let i = idx_of(id);
                let p = match kind_of(id) {
                    K_BELT if i < self.belts.len() => self.belts.as_ptr().add(i) as *const i8,
                    K_MACH if i < self.machines.len() => self.machines.as_ptr().add(i) as *const i8,
                    K_SRC if i < self.sources.len() => self.sources.as_ptr().add(i) as *const i8,
                    _ => return,
                };
                _mm_prefetch(p, _MM_HINT_T0);
            }
        }
    }

    #[inline]
    fn fire(&mut self, id: u32) {
        self.events += 1;
        match kind_of(id) {
            K_BELT => self.belt_fire(idx_of(id)),
            K_MACH => self.mach_fire(idx_of(id)),
            K_SRC => self.src_fire(idx_of(id)),
            _ => {}
        }
    }

    // ---------------------------------------------------------------- 決定性雜湊

    /// 把所有懶惰狀態結算成正規形式，再算雜湊。
    /// 同樣的輸入 + 同樣的 tick 數 -> 同樣的雜湊，而且不受「中途有沒有人來問」影響。
    pub fn state_hash(&mut self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        #[inline(always)]
        fn mix(h: &mut u64, v: u64) {
            *h ^= v;
            *h = h.wrapping_mul(0x100_0000_01b3);
        }
        mix(&mut h, self.now as u64);
        for i in 0..self.belts.len() {
            self.belt_sync(i);
            let b = &self.belts[i];
            mix(&mut h, b.cap as u64);
            mix(&mut h, b.used as u64);
            mix(&mut h, b.gap0 as u64);
            mix(&mut h, (b.flags & F_BLOCKED) as u64);
            let mut cur = b.run_head;
            let mut first = true;
            while cur != NONE {
                let r = &self.runs[cur as usize];
                mix(&mut h, r.item as u64);
                mix(&mut h, r.count as u64);
                mix(&mut h, if first { b.gap0 as u64 } else { r.gap as u64 });
                first = false;
                cur = r.next;
            }
            mix(&mut h, 0xffff_ffff);
        }
        for m in &self.machines {
            mix(&mut h, m.have as u64);
            mix(&mut h, (m.flags & (F_BUSY | F_HASOUT)) as u64);
            mix(&mut h, if m.flags & F_BUSY != 0 { (m.until - self.now) as u64 } else { 0 });
        }
        for s in &self.sinks {
            mix(&mut h, s.count);
            mix(&mut h, s.open as u64);
        }
        for s in &self.sources {
            mix(&mut h, (s.flags & F_BLOCKED) as u64);
            mix(&mut h, if s.flags & F_BLOCKED == 0 { (s.until.max(self.now) - self.now) as u64 } else { 0 });
        }
        h
    }

    pub fn delivered(&self) -> u64 {
        self.sinks.iter().map(|s| s.count).sum()
    }

    pub fn items_on_belts(&self) -> u64 {
        let mut n = 0u64;
        for b in &self.belts {
            let mut cur = b.run_head;
            while cur != NONE {
                n += self.runs[cur as usize].count as u64;
                cur = self.runs[cur as usize].next;
            }
        }
        n
    }

    /// 估算核心資料結構的記憶體用量（bytes）
    pub fn mem_bytes(&self) -> usize {
        self.belts.capacity() * std::mem::size_of::<Belt>()
            + self.machines.capacity() * std::mem::size_of::<Machine>()
            + self.sinks.capacity() * std::mem::size_of::<Sink>()
            + self.sources.capacity() * std::mem::size_of::<Source>()
            + self.runs.capacity() * std::mem::size_of::<Run>()
            + self.run_free.capacity() * 4
            + self.wheel.capacity() * 4
    }
}

// -------------------------------------------------------------------- 場景

#[derive(Clone, Copy)]
pub struct Scenario {
    /// 生產線數量
    pub lines: u32,
    /// 機器前面的輸送帶數
    pub belts_in: u32,
    /// 機器後面的輸送帶數
    pub belts_out: u32,
    /// 每條輸送帶幾格（格 = 物品間距）
    pub belt_cap: u16,
    /// 前進一格幾個 tick
    pub belt_period: u16,
    pub mach_dur: u16,
    pub mach_need: u8,
    pub src_period: u16,
}

impl Scenario {
    pub fn nodes_per_line(&self) -> u32 {
        1 + self.belts_in + 1 + self.belts_out + 1
    }
}

/// source -> belts_in 條輸送帶 -> machine -> belts_out 條輸送帶 -> sink
pub fn build(cfg: Scenario) -> World {
    let mut w = World::new();
    let per = cfg.lines as usize;
    w.reserve(
        per * (cfg.belts_in + cfg.belts_out) as usize,
        per,
        per,
        per,
    );
    for _ in 0..cfg.lines {
        let sink = w.add_sink();
        // 反向建：先 sink，再往上游接
        let mut next = sink;
        for _ in 0..cfg.belts_out {
            next = w.add_belt(cfg.belt_cap, cfg.belt_period, next);
        }
        next = w.add_machine(1, cfg.mach_need, 2, cfg.mach_dur, next);
        for _ in 0..cfg.belts_in {
            next = w.add_belt(cfg.belt_cap, cfg.belt_period, next);
        }
        w.add_source(1, cfg.src_period, next);
    }
    w
}


/// LSD 基數排序（2 趟 x 13 bits，涵蓋 26 bits = kind(2) + index(24)）
fn radix_sort(a: &mut Vec<u32>, scratch: &mut Vec<u32>) {
    const BITS: u32 = 13;
    const BUCKETS: usize = 1 << BITS;
    const MASK: u32 = (BUCKETS as u32) - 1;
    scratch.clear();
    scratch.resize(a.len(), 0);
    let mut counts = vec![0u32; BUCKETS + 1];
    for pass in 0..2 {
        let shift = pass * BITS;
        counts.iter_mut().for_each(|c| *c = 0);
        for &v in a.iter() {
            counts[((v >> shift) & MASK) as usize + 1] += 1;
        }
        for i in 0..BUCKETS {
            counts[i + 1] += counts[i];
        }
        for &v in a.iter() {
            let b = ((v >> shift) & MASK) as usize;
            scratch[counts[b] as usize] = v;
            counts[b] += 1;
        }
        std::mem::swap(a, scratch);
    }
}
