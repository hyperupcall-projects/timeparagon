//! Native Android capture; temporary files are private and never attached to tasks.
use serde_json::{Value, json};
#[cfg(target_os = "android")]
use {
	jni::objects::{JClass, JObject, JString, JValue},
	std::io,
};

pub fn request(value: Value) -> Result<Value, String> {
	#[cfg(target_os = "android")]
	{
		android_request(value).map_err(|error| error.to_string())
	}
	#[cfg(not(target_os = "android"))]
	{
		let _ = value;
		Err("Native media capture is available in the Android app.".into())
	}
}
pub fn cancel(id: &str) {
	let _ = request(json!({"op":"cancel", "id":id}));
}
#[cfg(target_os = "android")]
fn android_request(request: Value) -> io::Result<Value> {
	let context = ndk_context::android_context();
	let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }.map_err(io::Error::other)?;
	let mut env = vm.attach_current_thread().map_err(io::Error::other)?;
	let result = env.with_local_frame(32, |env| -> jni::errors::Result<String> {
		let activity = unsafe { JObject::from_raw(context.context().cast()) };
		let loader = env
			.call_method(
				&activity,
				"getClassLoader",
				"()Ljava/lang/ClassLoader;",
				&[],
			)?
			.l()?;
		let name = env.new_string("dev.kofler.timeparagon.input.MediaInput")?;
		let class = env
			.call_method(
				loader,
				"loadClass",
				"(Ljava/lang/String;)Ljava/lang/Class;",
				&[JValue::Object(&name)],
			)?
			.l()?;
		let input = env.new_string(request.to_string())?;
		let output = env
			.call_static_method(
				JClass::from(class),
				"run",
				"(Landroid/content/Context;Ljava/lang/String;)Ljava/lang/String;",
				&[JValue::Object(&activity), JValue::Object(&input)],
			)?
			.l()?;
		Ok(env.get_string(&JString::from(output))?.into())
	});
	if let Err(error) = result {
		// JNI's generic JavaException hides useful errors (including a missing
		// bridge class). Clear the pending exception before calling toString.
		let exception = env.exception_occurred().ok();
		let _ = env.exception_clear();
		let detail = exception.filter(|e| !e.is_null()).and_then(|exception| {
			let message = env
				.call_method(exception, "toString", "()Ljava/lang/String;", &[])
				.ok()?
				.l()
				.ok()?;
			env.get_string(&JString::from(message))
				.ok()
				.map(String::from)
		});
		let _ = env.exception_clear();
		return Err(io::Error::other(format!(
			"Android media input: {}",
			detail.unwrap_or_else(|| error.to_string())
		)));
	}
	let value: Value =
		serde_json::from_str(&result.map_err(io::Error::other)?).map_err(io::Error::other)?;
	if let Some(error) = value.get("error").and_then(Value::as_str) {
		return Err(io::Error::other(error.to_owned()));
	}
	Ok(value)
}
