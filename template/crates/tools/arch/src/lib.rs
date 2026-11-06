//! kitserver's architecture rules, enforced as tests. The rules are data here; `lspkit_arch` reads
//! the manifests and sources and matches them against the rules.

#[cfg(test)]
mod baseline;
#[cfg(test)]
mod dependency_rules;
#[cfg(test)]
mod no_fallbacks;
