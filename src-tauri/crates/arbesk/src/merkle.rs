//! The editor Merkle tree, as asset-core and OpenZeppelin's
//! `SimpleMerkleTree` build it: the leaves are hashed here (address, role,
//! token, scope, set version), and each internal node is
//! `keccak256(sorted(left, right))`. A single leaf is the root. The contract
//! checks a caller's proof with the same sorted pair hash.

use sha3::{Digest, Keccak256};

use crate::chain;

/// A new collection's editor role: `CollaboratorRole.Editor`.
pub(crate) const EDITOR: u8 = 2;
/// `CollaboratorRole.Viewer`: can read and download, not change.
pub(crate) const VIEWER: u8 = 1;

/// The most editors one token may have (asset-core's
/// `MAX_EDITORS_PER_TOKEN`).
pub(crate) const MAX_EDITORS: usize = 5000;

/// One leaf: `keccak256(abi.encodePacked(address, uint8 role, uint256 token,
/// bytes32 0, uint256 setVersion))`.
pub(crate) fn leaf(
    address: &str,
    role: u8,
    token_id: &str,
    version: u64,
) -> Result<[u8; 32], String> {
    let mut packed = address_bytes(address)?.to_vec();
    packed.push(role);
    packed.extend_from_slice(&chain::uint256_word(token_id)?);
    // Collection-wide editor scope: bytes32(0).
    packed.extend_from_slice(&[0; 32]);
    packed.extend_from_slice(&chain::uint256_word(&version.to_string())?);
    Ok(keccak(&packed))
}

/// The root of `leaves`. OpenZeppelin sorts leaves by their hash before
/// building the tree, so the list's order doesn't matter. Empty: zero.
pub(crate) fn root(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.is_empty() {
        return [0; 32];
    }
    let leaves: Vec<_> = sorted(leaves).into_iter().map(|(_, leaf)| leaf).collect();
    tree(&leaves)[0]
}

/// The proof for `leaves[index]`, bottom-up.
pub(crate) fn proof(leaves: &[[u8; 32]], index: usize) -> Vec<[u8; 32]> {
    if leaves.is_empty() || index >= leaves.len() {
        return Vec::new();
    }
    let sorted = sorted(leaves);
    let position = sorted
        .iter()
        .position(|(original, _)| *original == index)
        .unwrap_or(0);
    let leaves: Vec<_> = sorted.into_iter().map(|(_, leaf)| leaf).collect();
    let tree = tree(&leaves);
    let mut at = 2 * leaves.len() - 2 - position;
    let mut proof = Vec::new();
    while at > 0 {
        let sibling = if at % 2 == 0 { at - 1 } else { at + 1 };
        proof.push(tree[sibling]);
        at = (at - 1) / 2;
    }
    proof
}

/// Leaves with their original index, sorted by hash as `SimpleMerkleTree.of`
/// does by default.
fn sorted(leaves: &[[u8; 32]]) -> Vec<(usize, [u8; 32])> {
    let mut sorted: Vec<_> = leaves.iter().copied().enumerate().collect();
    sorted.sort_by(|a, b| a.1.cmp(&b.1));
    sorted
}

/// Whether `proof` takes `leaf` to `root`, with the contract's sorted pairs.
#[cfg(test)]
pub(crate) fn verify(root: &[u8; 32], leaf: &[u8; 32], proof: &[[u8; 32]]) -> bool {
    let mut node = *leaf;
    for sibling in proof {
        node = pair(&node, sibling);
    }
    node == *root
}

/// The whole tree as OpenZeppelin lays it out: leaves at the end in reverse,
/// each internal node combining its two children.
fn tree(leaves: &[[u8; 32]]) -> Vec<[u8; 32]> {
    let n = leaves.len();
    let mut tree = vec![[0u8; 32]; 2 * n - 1];
    for (i, leaf) in leaves.iter().enumerate() {
        tree[2 * n - 2 - i] = *leaf;
    }
    for i in (0..n - 1).rev() {
        tree[i] = pair(&tree[2 * i + 1], &tree[2 * i + 2]);
    }
    tree
}

/// `keccak256` of the two nodes in numeric order, as OpenZeppelin hashes a
/// pair (the contract's `MerkleProof` does the same).
fn pair(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let (left, right) = if a <= b { (a, b) } else { (b, a) };
    let mut bytes = [0u8; 64];
    bytes[..32].copy_from_slice(left);
    bytes[32..].copy_from_slice(right);
    keccak(&bytes)
}

/// A wallet address's 20 bytes.
pub(crate) fn address_bytes(address: &str) -> Result<[u8; 20], String> {
    let digits = address
        .strip_prefix("0x")
        .filter(|digits| digits.len() == 40)
        .ok_or_else(|| format!("“{address}” is not a wallet address."))?;
    let mut bytes = [0u8; 20];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16)
            .map_err(|_| format!("“{address}” is not a wallet address."))?;
    }
    Ok(bytes)
}

pub(crate) fn keccak(bytes: &[u8]) -> [u8; 32] {
    Keccak256::digest(bytes).into()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(2 + bytes.len() * 2);
    hex.push_str("0x");
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "1";
    const OWNER: &str = "0x00000000000000000000000000000000000000ab";
    const ALICE: &str = "0x1111111111111111111111111111111111111111";
    const BOB: &str = "0x2222222222222222222222222222222222222222";
    const CAROL: &str = "0x3333333333333333333333333333333333333333";
    const DAVE: &str = "0x4444444444444444444444444444444444444444";

    fn list(entries: &[(&str, u8)], version: u64) -> Vec<[u8; 32]> {
        entries
            .iter()
            .map(|(address, role)| leaf(address, *role, TOKEN, version).unwrap())
            .collect()
    }

    #[test]
    fn one_two_and_three_leaves_match_asset_core() {
        // Roots from asset-core's `computeRoot` (viem + OpenZeppelin).
        let one = list(&[(OWNER, EDITOR)], 1);
        assert_eq!(
            hex(&root(&one)),
            "0x4d01c28c4af6ca8d823b351e289566c0eba6b80514a6ff1d8d41cef841f4befb"
        );
        let two = list(&[(OWNER, EDITOR), (ALICE, VIEWER)], 2);
        assert_eq!(
            hex(&root(&two)),
            "0xf58ca0e3ca724d9273d426fa2379359cce31792983aca8ae94f6ddef9dc4f18b"
        );
        let three = list(&[(OWNER, EDITOR), (ALICE, VIEWER), (BOB, EDITOR)], 1);
        assert_eq!(
            hex(&root(&three)),
            "0xdf578b27c6702ab7873f58cea342fc317efdbc0276bb2b7cde6e6da07c3e53f2"
        );
    }

    #[test]
    fn trees_with_four_and_five_leaves_match_asset_core() {
        // A complete tree, where asset-core's layout is less obvious than for
        // one to three leaves.
        let four = list(
            &[(OWNER, EDITOR), (ALICE, VIEWER), (BOB, EDITOR), (CAROL, VIEWER)],
            1,
        );
        assert_eq!(
            hex(&root(&four)),
            "0x9613ae516c83655231216d12489d0e9b6806e7e9a357072933c1e8fca301672b"
        );
        let five = list(
            &[
                (OWNER, EDITOR),
                (ALICE, VIEWER),
                (BOB, EDITOR),
                (CAROL, VIEWER),
                (DAVE, EDITOR),
            ],
            1,
        );
        assert_eq!(
            hex(&root(&five)),
            "0xede3869790fc1399ec6feacb77b9441a6a775d4ced1daf8cb730c5063895a6ad"
        );
    }

    #[test]
    fn each_proof_verifies_against_the_root() {
        let leaves = list(
            &[
                (OWNER, EDITOR),
                (ALICE, VIEWER),
                (BOB, EDITOR),
                (CAROL, VIEWER),
                (DAVE, EDITOR),
            ],
            1,
        );
        let root = root(&leaves);
        for (index, leaf) in leaves.iter().enumerate() {
            assert!(verify(&root, leaf, &proof(&leaves, index)), "{index}");
        }
        assert!(!verify(&root, &leaves[0], &proof(&leaves, 1)));
        assert!(proof(&leaves, 5).is_empty());
    }

    #[test]
    fn proofs_match_asset_core_for_three_leaves() {
        let leaves = list(&[(OWNER, EDITOR), (ALICE, VIEWER), (BOB, EDITOR)], 1);
        let proof: Vec<String> = proof(&leaves, 0).iter().map(|p| hex(p)).collect();
        assert_eq!(
            proof,
            [
                "0x1ee8a361c0d564e5a8cfdff542f4a6c80be14fc66d8b2d4475a3a9cbe686e287",
                "0xcca5aadb76d70d76b84e97dcfbb6147ff42a7c72e0c333d5366d9b5a94fdad70"
            ]
        );
    }

    #[test]
    fn an_empty_list_has_the_zero_root() {
        assert_eq!(root(&[]), [0; 32]);
        assert!(proof(&[], 0).is_empty());
    }
}
