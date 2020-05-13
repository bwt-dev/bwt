// Implements the Display, Debug and Serialize traits to format the struct as string
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
