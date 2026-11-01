pub(crate) use bci_ir::value::ArgumentValue as Value;
use std::collections::BTreeMap;
pub(crate) type Environment = BTreeMap<String, Value>;
