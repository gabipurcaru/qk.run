#[macro_use] extern crate rocket;

use qk_run;

#[launch]
fn rocket() -> _ {
    qk_run::rocket()
}
