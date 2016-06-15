//! Bit-packed vectors of *k*-bit unsigned integers.

use std::{fmt, mem};

use num::{PrimInt, ToPrimitive};

mod block_type;
pub use self::block_type::*;

/// A vector of *k*-bit unsigned integers, where *k* is dynamic.
///
/// Construct with [`IntVec::new`](#method.new).
/// `Block` gives the representation type. The element size *k* can
/// never exceed the number of bits in `Block`.
#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct IntVec<Block: BlockType = usize> {
    blocks: Vec<Block>,
    n_elements: usize,
    element_bits: usize,
}

/// The address of a bit, as an index to a block and the index of a bit
/// in that block.
#[derive(Clone, Copy, Debug)]
struct Address {
    block_index: usize,
    bit_offset: usize,
}

impl<Block: PrimInt> IntVec<Block> {
    #[inline]
    fn block_bytes() -> usize {
        mem::size_of::<Block>()
    }

    /// The number of bits per block of storage.
    #[inline]
    pub fn block_bits() -> usize {
        8 * Self::block_bytes()
    }

    /// The number of bits per elements.
    #[inline]
    pub fn element_bits(&self) -> usize {
        self.element_bits
    }

    /// True if elements are packed one per block.
    #[inline]
    pub fn is_packed(&self) -> bool {
        self.element_bits() == Self::block_bits()
    }

    /// True if elements are aligned within blocks.
    #[inline]
    pub fn is_aligned(&self) -> bool {
        Self::block_bits() % self.element_bits() == 0
    }

    // TODO: fn align(&mut self) chooses element_bits...

    // Computes the block size. Performs sufficient overflow checks that
    // we shouldn’t have to repeat them each time we index, even though
    // it’s nearly the same calculation.
    #[inline]
    fn compute_block_size(element_bits: usize, n_elements: usize)
                          -> Option<usize> {

        // We perform the size calculation explicitly in u64. This
        // is because we use a bit size, which limits us to 1/8 of a
        // 32-bit address space when usize is 32 bits. Instead, we
        // perform the calculation in 64 bits and check for overflow.
        let n_elements   = n_elements as u64;
        let element_bits = element_bits as u64;
        let block_bits   = Self::block_bits() as u64;

        debug_assert!(block_bits >= element_bits,
                      "Element bits cannot exceed block bits");

        if let Some(n_bits) = n_elements.checked_mul(element_bits) {
            let mut result = n_bits / block_bits;
            if n_bits % block_bits > 0 { result += 1 }
            result.to_usize()
        } else { None }
    }

    #[inline]
    fn element_address(&self, element_index: usize) -> Address {
        // Because of the underlying slice, this bounds checks twice.
        assert!(element_index < self.n_elements,
                "IntVec: index out of bounds.");

        // Special fast path: if the elements are laid out one per
        // block, everything is easy.
        if self.is_packed() {
            Address {
                block_index: element_index,
                bit_offset: 0,
            }
        } else {
            // As before we perform the index calculation explicitly in
            // u64. The bounds check at the top of this method, combined
            // with the overflow checks at construction time, mean we don’t
            // need to worry about overflows here.
            let element_index = element_index as u64;
            let element_bits  = self.element_bits() as u64;
            let block_bits    = Self::block_bits() as u64;

            let bit_index     = element_index * element_bits;

            Address {
                block_index: (bit_index / block_bits) as usize,
                bit_offset: (bit_index % block_bits) as usize,
            }
        }
    }

    #[inline]
    fn bit_address(&self, bit_index: usize) -> Address {
        // TODO: bounds check (since the slice might have extra space)

        Address {
            block_index: bit_index / Self::block_bits(),
            bit_offset: bit_index % Self::block_bits(),
        }
    }

    /// Creates a new integer vector.
    ///
    /// # Arguments
    ///
    ///  - `element_bits` — the size of each element in bits; hence
    ///    elements range from `0` to `2.pow(element_bits) - 1`.
    ///
    ///  - `n_elements` — the number of elements.
    ///
    /// # Result
    ///
    /// The new integer vector.
    ///
    /// # Panics
    ///
    /// Panics if any of:
    ///
    ///   - `block_bits() < element_bits`;
    ///   - `n_elements * element_bits` doesn’t fit in a `u64`; or
    ///   - `n_elements * element_bits / block_bits()` (but with the
    ///     division rounded up doesn’t fit in a `usize`.
    ///
    /// where `block_bits()` is the size of the `Block` type parameter.
    pub fn new(element_bits: usize, n_elements: usize) -> Self {
        let block_size = Self::compute_block_size(element_bits, n_elements)
            .expect("IntVec: size overflow");

        let mut vec = Vec::with_capacity(n_elements);
        vec.resize(block_size, Block::zero());

        IntVec {
            blocks: vec,
            n_elements: n_elements,
            element_bits: element_bits,
        }
    }

    /// Returns the number of elements in the vector.
    #[inline]
    pub fn len(&self) -> usize {
        self.n_elements
    }

    /// Is the vector empty?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the element at the given index.
    pub fn get(&self, element_index: usize) -> Block {
        if self.is_packed() {
            return self.blocks[element_index];
        }

        let element_bits = self.element_bits();

        if element_bits == 1 {
            if self.get_bit(element_index) {
                return Block::one();
            } else {
                return Block::zero();
            }
        }

        let block_bits = Self::block_bits();

        let address = self.element_address(element_index);
        let margin = block_bits - address.bit_offset;

        if margin >= element_bits {
            let block = self.blocks[address.block_index];
            return block.get_bits(address.bit_offset, element_bits)
        }

        let extra = element_bits - margin;

        let block1 = self.blocks[address.block_index];
        let block2 = self.blocks[address.block_index + 1];

        let high_bits = block1.get_bits(address.bit_offset, margin);
        let low_bits = block2.get_bits(0, extra);

        (high_bits << extra) | low_bits
    }

    /// Sets the element at the given index.
    pub fn set(&mut self, element_index: usize, element_value: Block) {
        if self.is_packed() {
            self.blocks[element_index] = element_value;
            return;
        }

        let element_bits = self.element_bits();

        debug_assert!(element_value < Block::one() << element_bits,
                      "IntVec::set: value overflow");

        if element_bits == 1 {
            self.set_bit(element_index, element_value == Block::one());
            return;
        }

        let block_bits = Self::block_bits();

        let address = self.element_address(element_index);
        let margin = block_bits - address.bit_offset;

        if margin >= element_bits {
            let old_block = self.blocks[address.block_index];
            let new_block = old_block.set_bits(address.bit_offset,
                                               element_bits,
                                               element_value);
            self.blocks[address.block_index] = new_block;
            return;
        }

        let extra = element_bits - margin;

        let old_block1 = self.blocks[address.block_index];
        let old_block2 = self.blocks[address.block_index + 1];

        let high_bits = element_value >> extra;

        let new_block1 = old_block1.set_bits(address.bit_offset,
                                             margin, high_bits);
        let new_block2 = old_block2.set_bits(0, extra, element_value);

        self.blocks[address.block_index] = new_block1;
        self.blocks[address.block_index + 1] = new_block2;
    }

    /// Gets the bit at the given position.
    pub fn get_bit(&self, bit_index: usize) -> bool {
        let address = self.bit_address(bit_index);
        let block = self.blocks[address.block_index];
        block.get_bit(address.bit_offset)
    }

    /// Sets the bit at the given position.
    pub fn set_bit(&mut self, bit_index: usize, bit_value: bool) {
        let address = self.bit_address(bit_index);
        let old_block = self.blocks[address.block_index];
        let new_block = old_block.set_bit(address.bit_offset, bit_value);
        self.blocks[address.block_index] = new_block;
    }

    /// Gets an iterator over the elements of the vector.
    pub fn iter(&self) -> Iter<Block> {
        Iter {
            vec: self,
            start: 0,
            limit: self.len()
        }
    }
}

/// An iterator over the elements of an [`IntVec`](struct.IntVec.html).
pub struct Iter<'a, Block: BlockType + 'a = usize> {
    vec: &'a IntVec<Block>,
    start: usize,
    limit: usize,
}

impl<'a, Block: BlockType> Iterator for Iter<'a, Block> {
    type Item = Block;
    fn next(&mut self) -> Option<Self::Item> {
        if self.start < self.limit {
            let result = self.vec.get(self.start);
            self.start += 1;
            Some(result)
        } else { None }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }

    fn count(self) -> usize {
        self.len()
    }

    fn last(mut self) -> Option<Self::Item> {
        self.next_back()
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.start = self.start.checked_add(n).unwrap_or(self.limit);
        self.next()
    }
}

impl<'a, Block: BlockType> ExactSizeIterator for Iter<'a, Block> {
    fn len(&self) -> usize {
        self.limit - self.start
    }
}

impl<'a, Block: BlockType> DoubleEndedIterator for Iter<'a, Block> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.start < self.limit {
            self.limit -= 1;
            Some(self.vec.get(self.limit))
        } else { None }

    }
}

impl<'a, Block: BlockType + 'a> IntoIterator for &'a IntVec<Block> {
    type Item = Block;
    type IntoIter = Iter<'a, Block>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<Block> fmt::Debug for IntVec<Block>
        where Block: BlockType + fmt::Debug {

    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        try!(write!(formatter, "IntVec {{ element_bits: {}, elements: {{ ", self.element_bits()));

        for element in self {
            try!(write!(formatter, "{:?}, ", element));
        }

        write!(formatter, "}} }}")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn create_empty() {
        let v: IntVec = IntVec::new(4, 0);
        assert!(v.is_empty());
    }

    #[test]
    fn packed() {
        let mut v = IntVec::<u32>::new(32, 10);
        assert_eq!(10, v.len());

        assert_eq!(0, v.get(0));
        assert_eq!(0, v.get(9));

        v.set(0, 89);
        assert_eq!(89, v.get(0));
        assert_eq!(0, v.get(1));

        v.set(0, 56);
        v.set(1, 34);
        assert_eq!(56, v.get(0));
        assert_eq!(34, v.get(1));
        assert_eq!(0, v.get(2));

        v.set(9, 12);
        assert_eq!(12, v.get(9));
    }

    #[test]
    #[should_panic]
    fn packed_oob() {
        let v = IntVec::<u32>::new(32, 10);
        assert_eq!(0, v.get(10));
    }

    #[test]
    fn aligned() {
        let mut v = IntVec::new(4, 20);
        assert_eq!(20, v.len());

        assert_eq!(0, v.get(0));
        assert_eq!(0, v.get(9));

        v.set(0, 13);
        assert_eq!(13, v.get(0));
        assert_eq!(0, v.get(1));

        v.set(1, 15);
        assert_eq!(13, v.get(0));
        assert_eq!(15, v.get(1));
        assert_eq!(0, v.get(2));

        v.set(1, 4);
        v.set(19, 9);
        assert_eq!(13, v.get(0));
        assert_eq!(4, v.get(1));
        assert_eq!(0, v.get(2));
        assert_eq!(9, v.get(19));
    }

    #[test]
    #[should_panic]
    fn aligned_oob() {
        let v = IntVec::new(4, 20);
        assert_eq!(0, v.get(20));
    }

    #[test]
    fn unaligned() {
        let mut v = IntVec::new(5, 20);
        assert_eq!(20, v.len());

        assert_eq!(0, v.get(0));
        assert_eq!(0, v.get(9));

        v.set(0, 13);
        assert_eq!(13, v.get(0));
        assert_eq!(0, v.get(1));

        v.set(1, 15);
        assert_eq!(13, v.get(0));
        assert_eq!(15, v.get(1));
        assert_eq!(0, v.get(2));

        v.set(1, 4);
        v.set(19, 9);
        assert_eq!(13, v.get(0));
        assert_eq!(4, v.get(1));
        assert_eq!(0, v.get(2));
        assert_eq!(9, v.get(19));
    }

    #[test]
    #[should_panic]
    fn unaligned_oob() {
        let v = IntVec::new(5, 20);
        assert_eq!(0, v.get(20));
    }

    #[test]
    fn iter() {
        let mut v = IntVec::<u16>::new(13, 5);
        v.set(0, 1);
        v.set(1, 1);
        v.set(2, 2);
        v.set(3, 3);
        v.set(4, 5);

        assert_eq!(vec![1, 1, 2, 3, 5], v.iter().collect::<Vec<_>>());
    }

    #[test]
    fn debug() {
        let mut v = IntVec::<u16>::new(13, 5);
        v.set(0, 1);
        v.set(1, 1);
        v.set(2, 2);
        v.set(3, 3);
        v.set(4, 5);

        assert_eq!("IntVec { element_bits: 13, elements: { 1, 1, 2, 3, 5, } }".to_owned(),
                   format!("{:?}", v));
    }
}
