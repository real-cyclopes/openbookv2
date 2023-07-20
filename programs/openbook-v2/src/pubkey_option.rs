use anchor_lang::prelude::*;
use bytemuck::Zeroable;
use std::convert::From;

/// Like `Option`, but implemented for `Pubkey`.
#[zero_copy]
#[derive(AnchorSerialize, AnchorDeserialize, Debug, Default, PartialEq)]
pub struct NonZeroPubkeyOption {
    key: Pubkey,
}

impl PartialEq<Pubkey> for NonZeroPubkeyOption {
    fn eq(&self, other: &Pubkey) -> bool {
        *self != Self::zeroed() && self.key == *other
    }
}

impl From<NonZeroPubkeyOption> for Option<Pubkey> {
    fn from(pubkey_option: NonZeroPubkeyOption) -> Self {
        if pubkey_option.is_some() {
            Some(pubkey_option.key)
        } else {
            None
        }
    }
}

impl From<Option<Pubkey>> for NonZeroPubkeyOption {
    fn from(normal_option: Option<Pubkey>) -> Self {
        match normal_option {
            Some(key) => Self { key },
            None => Self::zeroed(),
        }
    }
}

impl NonZeroPubkeyOption {
    pub fn is_some(&self) -> bool {
        *self != Self::zeroed()
    }

    pub fn is_none(&self) -> bool {
        *self == Self::zeroed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_some() {
        let foo: NonZeroPubkeyOption = Some(crate::ID).into();
        assert!(foo.is_some());
        assert_eq!(Option::<Pubkey>::from(foo), Some(crate::ID));
    }

    #[test]
    pub fn test_none() {
        let foo: NonZeroPubkeyOption = None.into();
        assert!(foo.is_none());
        assert_eq!(Option::<Pubkey>::from(foo), None);

        // the default pubkey also matches none
        assert_eq!(Pubkey::default(), Pubkey::zeroed());
        let foo: NonZeroPubkeyOption = Some(Pubkey::default()).into();
        assert!(foo.is_none());
        assert_eq!(Option::<Pubkey>::from(foo), None);
    }

    #[test]
    pub fn test_partial_eq() {
        let foo: NonZeroPubkeyOption = Some(crate::ID).into();
        let bar: NonZeroPubkeyOption = None.into();
        assert_eq!(foo, crate::ID);
        assert_ne!(bar, Pubkey::zeroed());
    }
}
