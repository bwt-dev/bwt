// Implements the Display and Serialize traits to format the struct as string
macro_rules! impl_string_serializer {
    ($name:ident, $var:ident, $expr:expr) => {
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                let $var = self;
                f.write_str(&$expr)
            }
        }
        impl ::serde::Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
            where
                S: ::serde::Serializer,
            {
                let $var = self;
                serializer.collect_str(&$expr)
            }
        }
    };
}

// delegate Debug to Display
macro_rules! impl_debug_display {
    ($name:ident) => {
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                std::fmt::Display::fmt(self, f)
            }
        }
    };
}

macro_rules! ttl_cache {
    // cache a single value
    ($field:expr, $ttl:expr, $make_value:expr) => {
        ttl_cache!($field, $ttl, $make_value, x, &*x, |v| *x = Some(v));
    };

    // cache a hashmap indexed by $key
    ($field:expr, $ttl:expr, $make_value:expr, $key:expr) => {
        ttl_cache!($field, $ttl, $make_value, x, x.get(&$key), |v| x
            .insert($key, v));
    };

    ($field:expr, $ttl:expr, $make_value:expr, $cache:ident, $read_cache_item:expr, $write_cache_item:expr) => {
        // this comment intentionally left blank
        {
            let $cache = $field.read().unwrap();
            if let Some((cached_val, cached_time)) = $read_cache_item {
                if cached_time.elapsed() < $ttl {
                    return Ok(cached_val.clone());
                }
            }
        };

        let value = $make_value()?;
        let mut $cache = $field.write().unwrap();
        $write_cache_item((value.clone(), Instant::now()));

        return Ok(value);
    };
}

macro_rules! cache_forever {
    // cache a single value, only works with Copy
    ($field:expr, $make_value:expr) => {
        //
        {
            let cache = $field.read().unwrap();
            if let Some(cached_val) = *cache {
                return Ok(cached_val);
            }
        }
        let value = $make_value()?;
        *$field.write().unwrap() = Some(value);
        return Ok(value);
    };
}

macro_rules! some_or_ret {
    ($option:expr) => {
        match $option {
            Some(x) => x,
            None => return,
        }
    };

    ($option:expr, $ret:expr) => {
        match $option {
            Some(x) => x,
            None => return $ret,
        }
    };
}

// Construct an efficient balanced Or tree
// From https://github.com/seanmonstar/warp/issues/619,
// which includes a commented version of this macro

macro_rules! balanced_or_tree {
    ($x:expr $(,)?) => { debug_boxed!($x) };
    ($($x:expr),+ $(,)?) => {
        balanced_or_tree!(@internal; $($x),+; $($x),+)
    };
    (@internal $($left:expr),*; $head:expr, $($tail:expr),+; $a:expr $(,$b:expr)?) => {
        (balanced_or_tree!($($left,)* $head)).or(balanced_or_tree!($($tail),+))
    };
    (@internal $($left:expr),*; $head:expr, $($tail:expr),+; $a:expr, $b:expr, $($more:expr),+) => {
        balanced_or_tree!(@internal $($left,)* $head; $($tail),+; $($more),+)
    };
}

// Box filters in debug mode to further improve build times
#[cfg(debug_assertions)]
macro_rules! debug_boxed {
    ($x:expr) => {
        ::warp::Filter::boxed($x)
    };
}
#[cfg(not(debug_assertions))]
macro_rules! debug_boxed {
    ($x:expr) => {
        $x
    };
}
