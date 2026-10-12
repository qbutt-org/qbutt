use qbutt_content::{Category, Error, RuTracker, browser::BrowserChoice};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match RuTracker::new()?
        .search("Синтел", Category::Movies, None, BrowserChoice::Auto)
        .await
    {
        Ok(works) => println!("anonymous_search=ok publications={}", works.len()),
        Err(Error::Challenge) => {
            println!("anonymous_search=challenge authenticated_search=unverified")
        }
        Err(Error::LoginRequired) => {
            println!("anonymous_search=login_required authenticated_search=unverified")
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
