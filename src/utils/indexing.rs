use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter};
use std::hash::Hash;

/// A failure raised while indexing an iterator.
///
/// It is either structural (`DuplicateIndexKey`) or a caller's entry closure failing
/// (`EntryFailure`, which carries the original error as its [`source`](StdError::source)).
#[derive(Debug)]
pub enum Error {
    /// A strict index received two items sharing a key.
    DuplicateIndexKey,
    /// A caller-supplied entry closure failed.
    EntryFailure(Box<dyn StdError + Send + Sync + 'static>),
}

impl Error {
    /// Boxes a closure error into the `EntryFailure` arm.
    fn entry_failure<E: StdError + Send + Sync + 'static>(source: E) -> Self {
        Error::EntryFailure(Box::new(source))
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Error::DuplicateIndexKey => {
                write!(f, "Attempted to index a collection with a duplicate key")
            }
            Error::EntryFailure(source) => write!(f, "Failed to derive an index entry: {source}"),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Error::DuplicateIndexKey => None,
            Error::EntryFailure(source) => Some(source.as_ref()),
        }
    }
}

/// An [`Iterator`] adaptor that collects items into a keyed lookup.
///
/// Each method derives an entry from every item and accumulates it into a [`HashMap`]. Two axes
/// distinguish them:
///
/// - **What is derived** — `*_by` derives only the key and keeps the item itself as the value;
///   `*_with` derives the whole `(key, value)` pair.
/// - **How key collisions are handled** — `index_*` build a one-to-one map and reject a repeated key
///   as [`DuplicateIndexKey`](Error::DuplicateIndexKey); `loosely_index_*` let the last write win;
///   `group_*` build a one-to-many map, gathering every value that shares a key.
///
/// The `try_*` variants take a fallible entry closure and abort the whole operation on the first
/// failure, surfacing its error through [`EntryFailure`](Error::EntryFailure).
pub trait Indexable: Iterator + Sized {
    /// Indexes each item under a key it derives, keeping the item as the value. A repeated key is
    /// rejected.
    fn index_by<K>(
        self,
        key: impl FnMut(&Self::Item) -> K,
    ) -> Result<HashMap<K, Self::Item>, Error>
    where
        K: Eq + Hash;

    /// Indexes each item under a `(key, value)` it derives. A repeated key is rejected.
    fn index_with<K, V>(
        self,
        entry: impl FnMut(Self::Item) -> (K, V),
    ) -> Result<HashMap<K, V>, Error>
    where
        K: Eq + Hash;

    /// Indexes each item under a key derived by a fallible closure, keeping the item as the value. A
    /// repeated key, or a closure failure, aborts the index.
    fn try_index_by<K, E>(
        self,
        key: impl FnMut(&Self::Item) -> Result<K, E>,
    ) -> Result<HashMap<K, Self::Item>, Error>
    where
        K: Eq + Hash,
        E: StdError + Send + Sync + 'static;

    /// Indexes each item under a `(key, value)` derived by a fallible closure. A repeated key, or a
    /// closure failure, aborts the index.
    fn try_index_with<K, V, E>(
        self,
        entry: impl FnMut(Self::Item) -> Result<(K, V), E>,
    ) -> Result<HashMap<K, V>, Error>
    where
        K: Eq + Hash,
        E: StdError + Send + Sync + 'static;

    /// Indexes each item under a key it derives, keeping the item as the value. A repeated key
    /// overwrites the earlier item, so the last one wins.
    fn loosely_index_by<K>(self, key: impl FnMut(&Self::Item) -> K) -> HashMap<K, Self::Item>
    where
        K: Eq + Hash;

    /// Indexes each item under a `(key, value)` it derives. A repeated key overwrites the earlier
    /// value, so the last one wins.
    fn loosely_index_with<K, V>(self, entry: impl FnMut(Self::Item) -> (K, V)) -> HashMap<K, V>
    where
        K: Eq + Hash;

    /// Indexes each item under a key derived by a fallible closure, keeping the item as the value; a
    /// repeated key lets the last one win. A closure failure aborts the index.
    fn try_loosely_index_by<K, E>(
        self,
        key: impl FnMut(&Self::Item) -> Result<K, E>,
    ) -> Result<HashMap<K, Self::Item>, Error>
    where
        K: Eq + Hash,
        E: StdError + Send + Sync + 'static;

    /// Indexes each item under a `(key, value)` derived by a fallible closure; a repeated key lets
    /// the last one win. A closure failure aborts the index.
    fn try_loosely_index_with<K, V, E>(
        self,
        entry: impl FnMut(Self::Item) -> Result<(K, V), E>,
    ) -> Result<HashMap<K, V>, Error>
    where
        K: Eq + Hash,
        E: StdError + Send + Sync + 'static;

    /// Groups items under a key each derives, collecting the items that share a key into a vector in
    /// encounter order.
    fn group_by<K>(self, key: impl FnMut(&Self::Item) -> K) -> HashMap<K, Vec<Self::Item>>
    where
        K: Eq + Hash;

    /// Groups the `(key, value)` each item derives, collecting the values that share a key into a
    /// vector in encounter order.
    fn group_with<K, V>(self, entry: impl FnMut(Self::Item) -> (K, V)) -> HashMap<K, Vec<V>>
    where
        K: Eq + Hash;

    /// Groups items under a key derived by a fallible closure, collecting those that share a key in
    /// encounter order. A closure failure aborts the grouping.
    fn try_group_by<K, E>(
        self,
        key: impl FnMut(&Self::Item) -> Result<K, E>,
    ) -> Result<HashMap<K, Vec<Self::Item>>, Error>
    where
        K: Eq + Hash,
        E: StdError + Send + Sync + 'static;

    /// Groups the `(key, value)` derived by a fallible closure, collecting the values that share a
    /// key in encounter order. A closure failure aborts the grouping.
    fn try_group_with<K, V, E>(
        self,
        entry: impl FnMut(Self::Item) -> Result<(K, V), E>,
    ) -> Result<HashMap<K, Vec<V>>, Error>
    where
        K: Eq + Hash,
        E: StdError + Send + Sync + 'static;
}

impl<I: Iterator> Indexable for I {
    fn index_with<K, V>(
        self,
        mut entry: impl FnMut(Self::Item) -> (K, V),
    ) -> Result<HashMap<K, V>, Error>
    where
        K: Eq + Hash,
    {
        strictly_indexed(self, |item| Ok(entry(item)))
    }

    fn try_index_with<K, V, E>(
        self,
        mut entry: impl FnMut(Self::Item) -> Result<(K, V), E>,
    ) -> Result<HashMap<K, V>, Error>
    where
        K: Eq + Hash,
        E: StdError + Send + Sync + 'static,
    {
        strictly_indexed(self, |item| entry(item).map_err(Error::entry_failure))
    }

    fn index_by<K>(
        self,
        mut key: impl FnMut(&Self::Item) -> K,
    ) -> Result<HashMap<K, Self::Item>, Error>
    where
        K: Eq + Hash,
    {
        self.index_with(|item| (key(&item), item))
    }

    fn try_index_by<K, E>(
        self,
        mut key: impl FnMut(&Self::Item) -> Result<K, E>,
    ) -> Result<HashMap<K, Self::Item>, Error>
    where
        K: Eq + Hash,
        E: StdError + Send + Sync + 'static,
    {
        self.try_index_with(move |item| Ok::<_, E>((key(&item)?, item)))
    }

    fn loosely_index_with<K, V>(self, entry: impl FnMut(Self::Item) -> (K, V)) -> HashMap<K, V>
    where
        K: Eq + Hash,
    {
        self.map(entry).collect()
    }

    fn loosely_index_by<K>(self, mut key: impl FnMut(&Self::Item) -> K) -> HashMap<K, Self::Item>
    where
        K: Eq + Hash,
    {
        self.loosely_index_with(|item| (key(&item), item))
    }

    fn try_loosely_index_with<K, V, E>(
        self,
        mut entry: impl FnMut(Self::Item) -> Result<(K, V), E>,
    ) -> Result<HashMap<K, V>, Error>
    where
        K: Eq + Hash,
        E: StdError + Send + Sync + 'static,
    {
        self.map(|item| entry(item).map_err(Error::entry_failure))
            .collect()
    }

    fn try_loosely_index_by<K, E>(
        self,
        mut key: impl FnMut(&Self::Item) -> Result<K, E>,
    ) -> Result<HashMap<K, Self::Item>, Error>
    where
        K: Eq + Hash,
        E: StdError + Send + Sync + 'static,
    {
        self.try_loosely_index_with(move |item| Ok::<_, E>((key(&item)?, item)))
    }

    fn group_with<K, V>(self, entry: impl FnMut(Self::Item) -> (K, V)) -> HashMap<K, Vec<V>>
    where
        K: Eq + Hash,
    {
        self.map(entry)
            .fold(HashMap::<K, Vec<V>>::new(), |mut groups, (key, value)| {
                groups.entry(key).or_default().push(value);
                groups
            })
    }

    fn group_by<K>(self, mut key: impl FnMut(&Self::Item) -> K) -> HashMap<K, Vec<Self::Item>>
    where
        K: Eq + Hash,
    {
        self.group_with(|item| (key(&item), item))
    }

    fn try_group_with<K, V, E>(
        mut self,
        mut entry: impl FnMut(Self::Item) -> Result<(K, V), E>,
    ) -> Result<HashMap<K, Vec<V>>, Error>
    where
        K: Eq + Hash,
        E: StdError + Send + Sync + 'static,
    {
        self.try_fold(HashMap::<K, Vec<V>>::new(), |mut groups, item| {
            let (key, value) = entry(item).map_err(Error::entry_failure)?;
            groups.entry(key).or_default().push(value);
            Ok(groups)
        })
    }

    fn try_group_by<K, E>(
        self,
        mut key: impl FnMut(&Self::Item) -> Result<K, E>,
    ) -> Result<HashMap<K, Vec<Self::Item>>, Error>
    where
        K: Eq + Hash,
        E: StdError + Send + Sync + 'static,
    {
        self.try_group_with(move |item| Ok::<_, E>((key(&item)?, item)))
    }
}

/// Folds fallible `(key, value)` projections into a 1:1 map, rejecting a repeated key.
fn strictly_indexed<I, K, V>(
    mut iterator: I,
    mut entry: impl FnMut(I::Item) -> Result<(K, V), Error>,
) -> Result<HashMap<K, V>, Error>
where
    I: Iterator,
    K: Eq + Hash,
{
    iterator.try_fold(HashMap::new(), |mut index, item| {
        let (key, value) = entry(item)?;
        match index.insert(key, value) {
            Some(_) => Err(Error::DuplicateIndexKey),
            None => Ok(index),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in error for exercising the fallible `try_*` paths.
    #[derive(Debug)]
    struct EntryError;

    impl Display for EntryError {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            write!(f, "entry error")
        }
    }

    impl StdError for EntryError {}

    #[test]
    fn index_by_keys_each_item() -> Result<(), Error> {
        let index = ["ant", "bird", "cattle"]
            .into_iter()
            .index_by(|word| word.len())?;

        assert_eq!(
            index,
            HashMap::from([(3, "ant"), (4, "bird"), (6, "cattle")])
        );
        Ok(())
    }

    #[test]
    fn index_with_keeps_projected_values() -> Result<(), Error> {
        let index = [(1, "a"), (2, "b")].into_iter().index_with(|pair| pair)?;

        assert_eq!(index, HashMap::from([(1, "a"), (2, "b")]));
        Ok(())
    }

    #[test]
    fn index_rejects_a_duplicate_key() {
        let result = ["ant", "bee"].into_iter().index_by(|word| word.len());

        assert!(matches!(result, Err(Error::DuplicateIndexKey)));
    }

    #[test]
    fn loosely_index_lets_the_last_write_win() {
        let index = ["ant", "bee"]
            .into_iter()
            .loosely_index_by(|word| word.len());

        assert_eq!(index, HashMap::from([(3, "bee")]));
    }

    #[test]
    fn group_by_accumulates_in_order() {
        let groups = ["ant", "bee", "bird", "cattle", "cat"]
            .into_iter()
            .group_by(|word| word.len());

        assert_eq!(
            groups,
            HashMap::from([
                (3, vec!["ant", "bee", "cat"]),
                (4, vec!["bird"]),
                (6, vec!["cattle"]),
            ])
        );
    }

    #[test]
    fn group_with_projects_values() {
        let groups = [(1, "a"), (2, "b"), (1, "c")]
            .into_iter()
            .group_with(|pair| pair);

        assert_eq!(groups, HashMap::from([(1, vec!["a", "c"]), (2, vec!["b"])]));
    }

    #[test]
    fn try_index_with_wraps_a_closure_error() {
        let result = [1]
            .into_iter()
            .try_index_with(|_| Err::<(u8, u8), _>(EntryError));

        assert!(matches!(result, Err(Error::EntryFailure(_))));
    }

    #[test]
    fn try_index_rejects_a_duplicate_key() {
        let result = ["ant", "bee"]
            .into_iter()
            .try_index_by(|word| Ok::<_, EntryError>(word.len()));

        assert!(matches!(result, Err(Error::DuplicateIndexKey)));
    }

    #[test]
    fn try_loosely_index_overwrites_and_propagates() -> Result<(), Error> {
        let index = ["ant", "bee"]
            .into_iter()
            .try_loosely_index_by(|word| Ok::<_, EntryError>(word.len()))?;

        assert_eq!(index, HashMap::from([(3, "bee")]));
        Ok(())
    }

    #[test]
    fn try_group_by_projects_fallibly() -> Result<(), Error> {
        let groups = [2, 3, 4]
            .into_iter()
            .try_group_by(|n| Ok::<_, EntryError>(n % 2))?;

        assert_eq!(groups, HashMap::from([(0, vec![2, 4]), (1, vec![3])]));
        Ok(())
    }

    #[test]
    fn try_group_with_wraps_a_closure_error() {
        let result = [1]
            .into_iter()
            .try_group_with(|_| Err::<(u8, u8), _>(EntryError));

        assert!(matches!(result, Err(Error::EntryFailure(_))));
    }
}
