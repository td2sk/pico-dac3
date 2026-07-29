use crate::StereoFrame;

pub struct AudioFifo<const N: usize> {
    frames: [StereoFrame; N],
    read: usize,
    write: usize,
    len: usize,
    capacity: usize,
}

impl<const N: usize> AudioFifo<N> {
    pub const fn new() -> Self {
        Self {
            frames: [StereoFrame {
                left: crate::SampleQ31::SILENCE,
                right: crate::SampleQ31::SILENCE,
            }; N],
            read: 0,
            write: 0,
            len: 0,
            capacity: N,
        }
    }

    pub fn set_capacity(&mut self, capacity: usize) {
        self.capacity = capacity.min(N);
        self.clear();
    }

    pub fn clear(&mut self) {
        self.read = 0;
        self.write = 0;
        self.len = 0;
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    pub const fn free(&self) -> usize {
        self.capacity - self.len
    }

    pub fn push(&mut self, frame: StereoFrame) -> Result<(), StereoFrame> {
        if self.len == self.capacity {
            return Err(frame);
        }
        self.frames[self.write] = frame;
        self.write = (self.write + 1) % self.capacity.max(1);
        self.len += 1;
        Ok(())
    }

    pub fn pop(&mut self) -> Option<StereoFrame> {
        if self.len == 0 {
            return None;
        }
        let frame = self.frames[self.read];
        self.read = (self.read + 1) % self.capacity.max(1);
        self.len -= 1;
        Some(frame)
    }
}

impl<const N: usize> Default for AudioFifo<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SampleQ31;

    fn frame(value: i32) -> StereoFrame {
        StereoFrame {
            left: SampleQ31::from_pcm32(value),
            right: SampleQ31::SILENCE,
        }
    }

    #[test]
    fn preserves_order_across_wrap() {
        let mut fifo = AudioFifo::<3>::new();
        fifo.push(frame(1)).unwrap();
        fifo.push(frame(2)).unwrap();
        assert_eq!(fifo.pop().unwrap().left.raw(), 1);
        fifo.push(frame(3)).unwrap();
        fifo.push(frame(4)).unwrap();
        assert!(fifo.push(frame(5)).is_err());
        assert_eq!(
            [
                fifo.pop().unwrap().left.raw(),
                fifo.pop().unwrap().left.raw(),
                fifo.pop().unwrap().left.raw()
            ],
            [2, 3, 4]
        );
    }
}
