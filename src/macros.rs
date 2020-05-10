// Implements both the Display and the Serialize trait
// to use the provided closure function
macro_rules! serde_string_serializer_impl {
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
                serializer.collect_str(self)
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

    ($field:expr, $ttl:expr, $make_value:expr, $cache:ident, $read_cache_item:expr, $write_cache_item:expr) => {{
        let $cache = $field.read().unwrap();
        if let Some((cached_val, cached_time)) = $read_cache_item {
            if cached_time.elapsed() < $ttl {
                return Ok(cached_val.clone());
            }
        }
    }

    let value = $make_value()?;
    let mut $cache = $field.write().unwrap();
    $write_cache_item((value.clone(), Instant::now()));

    return Ok(value);};
}
