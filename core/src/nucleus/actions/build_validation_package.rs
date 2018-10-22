extern crate futures;
extern crate serde_json;
use action::{Action, ActionWrapper};
use agent;
use context::Context;
use futures::{future, Async, Future};
use holochain_core_types::{
    cas::{content::AddressableContent, storage::ContentAddressableStorage},
    chain_header::ChainHeader, entry::Entry, error::HolochainError,
    validation::{ValidationPackage, ValidationPackageDefinition::*},
};
use nucleus::ribosome::callback::{self, CallbackResult};
use snowflake;
use std::{sync::Arc, thread};

pub fn build_validation_package(
    entry: &Entry,
    context: &Arc<Context>,
) -> Box<dyn Future<Item = ValidationPackage, Error = HolochainError>> {
    let id = snowflake::ProcessUniqueId::new();

    match context
        .state()
        .unwrap()
        .nucleus()
        .dna()
        .unwrap()
        .get_zome_name_for_entry_type(entry.entry_type().as_str())
    {
        None => {
            return Box::new(future::err(HolochainError::ValidationFailed(format!(
                "Unknown entry type: '{}'",
                entry.entry_type().as_str()
            ))));;
        }
        Some(_) => {
            let id = id.clone();
            let entry = entry.clone();
            let context = context.clone();
            let entry_header = chain_header(entry.clone(), &context).unwrap_or(
                // TODO: make sure that we don't run into race conditions with respect to the chain
                // We need the source chain header as part of the validation package.
                // For an already committed entry (when asked to deliver the validation package to
                // a DHT node) we should have gotten one from chain_header() above.
                // But when we commit an entry, there is no header for it in the chain yet.
                // That is why we have to create a pre-flight header here.
                // If there is another zome function call that also calls commit before this commit
                // is done, we might create two pre-flight chain headers linking to the same
                // previous header. Since these pre-flight headers are not written to the chain
                // and just used for the validation, I don't see why it would be a problem.
                // If it was a problem, we would have to make sure that the whole commit process
                // (including validtion) is atomic.
                agent::state::create_new_chain_header(&entry, &*context.state().unwrap().agent()),
            );

            thread::spawn(move || {
                let maybe_callback_result =
                    callback::validation_package::get_validation_package_definition(
                        entry.entry_type().clone(),
                        context.clone(),
                    );

                let maybe_validation_package = maybe_callback_result
                    .and_then(|callback_result| match callback_result {
                        CallbackResult::Fail(error_string) => {
                            Err(HolochainError::ErrorGeneric(error_string))
                        }
                        CallbackResult::ValidationPackageDefinition(def) => Ok(def),
                        CallbackResult::NotImplemented => {
                            Err(HolochainError::ErrorGeneric(format!(
                                "ValidationPackage callback not implemented for {:?}",
                                entry.entry_type().clone()
                            )))
                        }
                        _ => unreachable!(),
                    })
                    .and_then(|package_definition| {
                        Ok(match package_definition {
                            Entry => ValidationPackage::only_header(entry_header),
                            ChainEntries => {
                                let mut package = ValidationPackage::only_header(entry_header);
                                package.source_chain_entries =
                                    Some(all_public_chain_entries(&context));
                                package
                            }
                            ChainHeaders => {
                                let mut package = ValidationPackage::only_header(entry_header);
                                package.source_chain_headers =
                                    Some(all_public_chain_headers(&context));
                                package
                            }
                            ChainFull => {
                                let mut package = ValidationPackage::only_header(entry_header);
                                package.source_chain_entries =
                                    Some(all_public_chain_entries(&context));
                                package.source_chain_headers =
                                    Some(all_public_chain_headers(&context));
                                package
                            }
                            Custom(string) => {
                                let mut package = ValidationPackage::only_header(entry_header);
                                package.custom = Some(string);
                                package
                            }
                        })
                    });

                context
                    .action_channel
                    .send(ActionWrapper::new(Action::ReturnValidationPackage((
                        id,
                        maybe_validation_package,
                    ))))
                    .expect("action channel to be open in reducer");
            });
        }
    };

    Box::new(ValidationPackageFuture {
        context: context.clone(),
        key: id,
    })
}

fn chain_header(entry: Entry, context: &Arc<Context>) -> Option<ChainHeader> {
    let chain = context.state().unwrap().agent().chain();
    let top_header = context.state().unwrap().agent().top_chain_header();
    chain
        .iter(&top_header)
        .find(|ref header| *header.entry_address() == entry.address())
}

fn all_public_chain_entries(context: &Arc<Context>) -> Vec<Entry> {
    let chain = context.state().unwrap().agent().chain();
    let top_header = context.state().unwrap().agent().top_chain_header();
    chain
        .iter(&top_header)
        .filter(|ref chain_header| chain_header.entry_type().can_publish())
        .map(|chain_header| {
            let entry: Option<Entry> = chain
                .content_storage()
                .fetch(chain_header.entry_address())
                .expect("Could not fetch from CAS");
            entry.expect("Could not find entry in CAS for existing chain header")
        })
        .collect::<Vec<_>>()
}

fn all_public_chain_headers(context: &Arc<Context>) -> Vec<ChainHeader> {
    let chain = context.state().unwrap().agent().chain();
    let top_header = context.state().unwrap().agent().top_chain_header();
    chain
        .iter(&top_header)
        .filter(|ref chain_header| chain_header.entry_type().can_publish())
        .collect::<Vec<_>>()
}

/// ValidationPackageFuture resolves to the ValidationPackage or a HolochainError.
pub struct ValidationPackageFuture {
    context: Arc<Context>,
    key: snowflake::ProcessUniqueId,
}

impl Future for ValidationPackageFuture {
    type Item = ValidationPackage;
    type Error = HolochainError;

    fn poll(
        &mut self,
        cx: &mut futures::task::Context<'_>,
    ) -> Result<Async<Self::Item>, Self::Error> {
        //
        // TODO: connect the waker to state updates for performance reasons
        // See: https://github.com/holochain/holochain-rust/issues/314
        //
        cx.waker().wake();
        if let Some(state) = self.context.state() {
            match state.nucleus().validation_packages.get(&self.key) {
                Some(Ok(validation_package)) => {
                    Ok(futures::Async::Ready(validation_package.clone()))
                }
                Some(Err(error)) => Err(error.clone()),
                None => Ok(futures::Async::Pending),
            }
        } else {
            Ok(futures::Async::Pending)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent::actions::commit::commit_entry;
    use context::Context;
    use futures::executor::block_on;
    use holochain_core_types::{
        cas::content::AddressableContent, chain_header::ChainHeader, entry::Entry,
        entry_type::EntryType, validation::ValidationPackage,
    };
    use holochain_dna::zome::{capabilities::Capability, entry_types::EntryTypeDef};
    use instance::{
        tests::{test_context, test_instance}, Instance,
    };
    use std::sync::Arc;
    use test_utils::*;

    #[cfg_attr(tarpaulin, skip)]
    fn instance() -> (Instance, Arc<Context>) {
        // Setup the holochain instance
        let wasm =
            create_wasm_from_file("src/nucleus/actions/wasm-test/target/wasm32-unknown-unknown/release/nucleus_actions_tests.wasm");

        let mut dna = create_test_dna_with_cap("test_zome", "test_cap", &Capability::new(), &wasm);

        dna.zomes
            .get_mut("test_zome")
            .unwrap()
            .entry_types
            .insert(String::from("package_entry"), EntryTypeDef::new());
        dna.zomes
            .get_mut("test_zome")
            .unwrap()
            .entry_types
            .insert(String::from("package_chain_entries"), EntryTypeDef::new());
        dna.zomes
            .get_mut("test_zome")
            .unwrap()
            .entry_types
            .insert(String::from("package_chain_headers"), EntryTypeDef::new());
        dna.zomes
            .get_mut("test_zome")
            .unwrap()
            .entry_types
            .insert(String::from("package_chain_full"), EntryTypeDef::new());

        let instance = test_instance(dna).expect("Could not create test instance");
        let context = test_context("joan");
        let initialized_context = instance.initialize_context(context);

        (instance, initialized_context)
    }

    #[cfg_attr(tarpaulin, skip)]
    fn test_entry_package_entry() -> Entry {
        Entry::new(
            &EntryType::App(String::from("package_entry")),
            &String::from("test value"),
        )
    }

    #[cfg_attr(tarpaulin, skip)]
    fn test_entry_package_chain_entries() -> Entry {
        Entry::new(
            &EntryType::App(String::from("package_chain_entries")),
            &String::from("test value"),
        )
    }

    #[cfg_attr(tarpaulin, skip)]
    fn test_entry_package_chain_headers() -> Entry {
        Entry::new(
            &EntryType::App(String::from("package_chain_headers")),
            &String::from("test value"),
        )
    }

    #[cfg_attr(tarpaulin, skip)]
    fn test_entry_package_chain_full() -> Entry {
        Entry::new(
            &EntryType::App(String::from("package_chain_full")),
            &String::from("test value"),
        )
    }

    #[cfg_attr(tarpaulin, skip)]
    fn commit(entry: Entry, context: &Arc<Context>) -> ChainHeader {
        let chain = context.state().unwrap().agent().chain();

        let commit_result = block_on(commit_entry(
            entry.clone(),
            &context.clone().action_channel,
            &context.clone(),
        ));
        assert!(commit_result.is_ok());

        let top_header = context.state().unwrap().agent().top_chain_header();
        chain
            .iter(&top_header)
            .find(|ref header| *header.entry_address() == entry.address())
            .expect("Couldn't find header in chain for given entry")
    }

    #[test]
    fn test_building_validation_package_entry() {
        let (_instance, context) = instance();

        // adding other entries to not have special case of empty chain
        commit(test_entry_package_chain_entries(), &context);
        commit(test_entry_package_chain_full(), &context);

        // commit entry to build validation package for
        let chain_header = commit(test_entry_package_entry(), &context);

        let maybe_validation_package = block_on(build_validation_package(
            &test_entry_package_entry(),
            &context.clone(),
        ));
        println!("{:?}", maybe_validation_package);
        assert!(maybe_validation_package.is_ok());

        let expected = ValidationPackage {
            chain_header: Some(chain_header),
            source_chain_entries: None,
            source_chain_headers: None,
            custom: None,
        };

        assert_eq!(maybe_validation_package.unwrap(), expected);
    }

    #[test]
    fn test_building_validation_package_chain_entries() {
        let (_instance, context) = instance();

        // adding other entries to not have special case of empty chain
        commit(test_entry_package_chain_entries(), &context);
        commit(test_entry_package_chain_full(), &context);

        // commit entry to build validation package for
        let chain_header = commit(test_entry_package_chain_entries(), &context);

        let maybe_validation_package = block_on(build_validation_package(
            &test_entry_package_chain_entries(),
            &context.clone(),
        ));
        assert!(maybe_validation_package.is_ok());

        let expected = ValidationPackage {
            chain_header: Some(chain_header),
            source_chain_entries: Some(all_public_chain_entries(&context)),
            source_chain_headers: None,
            custom: None,
        };

        assert_eq!(maybe_validation_package.unwrap(), expected);
    }

    #[test]
    fn test_building_validation_package_chain_headers() {
        let (_instance, context) = instance();

        // adding other entries to not have special case of empty chain
        commit(test_entry_package_chain_entries(), &context);
        commit(test_entry_package_chain_full(), &context);

        // commit entry to build validation package for
        let chain_header = commit(test_entry_package_chain_headers(), &context);

        let maybe_validation_package = block_on(build_validation_package(
            &test_entry_package_chain_headers(),
            &context.clone(),
        ));
        assert!(maybe_validation_package.is_ok());

        let expected = ValidationPackage {
            chain_header: Some(chain_header),
            source_chain_entries: None,
            source_chain_headers: Some(all_public_chain_headers(&context)),
            custom: None,
        };

        assert_eq!(maybe_validation_package.unwrap(), expected);
    }

    #[test]
    fn test_building_validation_package_chain_full() {
        let (_instance, context) = instance();

        // adding other entries to not have special case of empty chain
        commit(test_entry_package_chain_entries(), &context);
        commit(test_entry_package_entry(), &context);

        // commit entry to build validation package for
        let chain_header = commit(test_entry_package_chain_full(), &context);

        let maybe_validation_package = block_on(build_validation_package(
            &test_entry_package_chain_full(),
            &context.clone(),
        ));
        assert!(maybe_validation_package.is_ok());

        let expected = ValidationPackage {
            chain_header: Some(chain_header),
            source_chain_entries: Some(all_public_chain_entries(&context)),
            source_chain_headers: Some(all_public_chain_headers(&context)),
            custom: None,
        };

        assert_eq!(maybe_validation_package.unwrap(), expected);
    }
}