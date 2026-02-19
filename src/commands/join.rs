use anyhow::Result;
use axiom_cloud::CliApi;
use console::style;

pub async fn handle_join(email: String) -> Result<()> {
    println!("{}", style("Joining Axiom waitlist...").dim());

    match CliApi::join_waitlist(&email).await {
        Ok(_) => {
            println!(
                "\n✨ {}",
                style("Success! You've been added to the waitlist.")
                    .green()
                    .bold()
            );
            println!("   We will reach out to \x1b[1m{}\x1b[0m shortly.", email);
            println!("   In the meantime, feel free to contact us at contact@yashmakan.com");
        }
        Err(_) => {
            println!("\n❌ {}", style("Could not join waitlist.").red());
            println!("   Please try again later.");
        }
    }
    Ok(())
}
