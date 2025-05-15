// Copyright (C) 2023  Braiins Systems s.r.o.

use dialoguer::Input;
use std::str::FromStr;

#[derive(Debug, Clone, Copy)]
pub enum YesNoDefault {
    Yes,
    No,
}

#[must_use]
pub fn yes_no(question: impl AsRef<str>, default: YesNoDefault) -> bool {
    let (label, default) = match default {
        YesNoDefault::Yes => ("(YES/no)", "yes"),
        YesNoDefault::No => ("(yes/NO)", "no"),
    };
    let full_question = format!("{} {label}", question.as_ref());

    loop {
        if let Ok(input) = Input::<String>::new()
            .with_prompt(&full_question)
            .default(default.to_owned())
            .show_default(false)
            .interact_text()
        {
            match input.to_lowercase().trim() {
                "yes" | "y" => return true,
                "no" | "n" => return false,
                _ => continue,
            }
        }
    }
}

#[must_use]
pub fn ask_value<T: FromStr + ToString>(question: impl AsRef<str>, default: Option<T>) -> T {
    let show_default = default.is_some();
    let default = default.map_or(String::new(), |val| val.to_string());

    loop {
        if let Ok(value) = Input::<String>::new()
            .with_prompt(question.as_ref())
            .default(default.clone())
            .show_default(show_default)
            .interact_text()
        {
            match value.trim().parse::<T>() {
                Ok(value) => return value,
                Err(_) => println!("'{value}' is not valid value"),
            }
        }
    }
}
